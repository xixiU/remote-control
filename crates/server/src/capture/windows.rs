//! Windows 屏幕捕获实现（基于 Desktop Duplication API）

use super::{CursorShape, Frame, ScreenCapturer, ScreenInfo};
use anyhow::{bail, Context, Result};
use tracing::{info, warn};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_DEBUG, D3D11_CREATE_DEVICE_FLAG,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, DXGI_ERROR_ACCESS_LOST,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTDUPL_POINTER_SHAPE_INFO, IDXGIDevice, IDXGIOutput, IDXGIResource,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT, DXGI_SAMPLE_DESC,
};

/// Windows 屏幕捕获器（Desktop Duplication API）
///
/// 使用 Desktop Duplication API 捕获桌面，GPU 加速零拷贝。
/// 注意：COM 对象在单线程中使用，通过 unsafe impl Send 允许跨线程传递。
pub struct WindowsCapturer {
    /// D3D11 设备
    device: ID3D11Device,
    /// 设备上下文
    context: ID3D11DeviceContext,
    /// 桌面复制对象
    duplication: Option<IDXGIOutputDuplication>,
    /// staging 纹理（CPU 可访问）
    staging_texture: Option<ID3D11Texture2D>,
    /// 屏幕信息
    info: ScreenInfo,
    /// 上一帧缓存（用于无新帧时返回）
    last_frame: Option<Frame>,
    /// 光标形状缓存
    cursor_cache: Option<CursorShape>,
    /// 光标序列号
    cursor_serial: u32,
    /// 显示器在 Virtual Screen 中的偏移
    _offset_x: i32,
    _offset_y: i32,
}

// SAFETY: WindowsCapturer 包含 COM 对象，这些对象本身不是 Send。
// 但我们确保：
// 1. WindowsCapturer 在单线程中使用（pipeline.rs 的独立线程）
// 2. 不会跨线程共享指针
// 3. COM 对象在 Drop 时会正确释放
unsafe impl Send for WindowsCapturer {}

impl WindowsCapturer {
    /// 创建 Windows 屏幕捕获器，捕获主显示器
    pub fn new() -> Result<Self> {
        // 1. 创建 D3D11 Device
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL_11_0;

        let create_flags: D3D11_CREATE_DEVICE_FLAG = D3D11_CREATE_DEVICE_DEBUG;

        unsafe {
            D3D11CreateDevice(
                None, // 默认适配器
                D3D_DRIVER_TYPE_HARDWARE,
                windows::Win32::Foundation::HMODULE::default(), // 无软件模块
                create_flags,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                7, // D3D11_SDK_VERSION
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut context),
            )
            .context("创建 D3D11 Device 失败")?;
        }

        let device = device.context("D3D11 Device 为空")?;
        let context = context.context("D3D11 DeviceContext 为空")?;

        info!("D3D11 Device 已创建，Feature Level: {:?}", feature_level);

        // 2. 获取 DXGI Device
        let dxgi_device: IDXGIDevice = {
            device
                .cast()
                .context("无法将 ID3D11Device 转换为 IDXGIDevice")?
        };

        // 3. 获取 DXGI Factory
        let adapter: IDXGIAdapter1 = unsafe {
            dxgi_device
                .GetParent()
                .context("无法获取 DXGI Adapter")?
        };
        let factory: IDXGIFactory1 = unsafe {
            adapter
                .GetParent()
                .context("无法获取 DXGI Factory")?
        };

        // 4. 枚举适配器，找到有输出的适配器
        let (duplication, output_desc, offset_x, offset_y) =
            unsafe { Self::create_duplication(&factory, &device)? };

        let info = ScreenInfo {
            width: (output_desc.DesktopCoordinates.right - output_desc.DesktopCoordinates.left) as u32,
            height: (output_desc.DesktopCoordinates.bottom - output_desc.DesktopCoordinates.top) as u32,
        };

        info!(
            "Windows 屏幕捕获器已初始化: {}x{}, Desktop Duplication API",
            info.width, info.height
        );

        Ok(Self {
            device,
            context,
            duplication: Some(duplication),
            staging_texture: None,
            info,
            last_frame: None,
            cursor_cache: None,
            cursor_serial: 0,
            _offset_x: offset_x,
            _offset_y: offset_y,
        })
    }

    /// 枚举适配器和输出，创建 OutputDuplication
    unsafe fn create_duplication(
        factory: &IDXGIFactory1,
        device: &ID3D11Device,
    ) -> Result<(IDXGIOutputDuplication, windows::Win32::Graphics::Dxgi::DXGI_OUTPUT_DESC, i32, i32)> {
        use windows::Win32::Graphics::Dxgi::DXGI_ERROR_NOT_FOUND;

        let mut adapter_idx = 0;
        loop {
            let adapter = match factory.EnumAdapters1(adapter_idx) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => {
                    bail!("未找到显示适配器")
                }
                Err(e) => {
                    bail!("枚举适配器失败: {:?}", e)
                }
            };

            // 枚举该适配器的所有输出
            let mut output_idx = 0;
            loop {
                let output: IDXGIOutput = match adapter.EnumOutputs(output_idx) {
                    Ok(o) => o,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => {
                        break; // 该适配器没有更多输出
                    }
                    Err(e) => {
                        warn!("枚举输出 {} 失败: {:?}", output_idx, e);
                        break;
                    }
                };

                // 获取输出描述
                let output1: IDXGIOutput1 = match output.cast() {
                    Ok(o) => o,
                    Err(e) => {
                        warn!("无法转换为 IDXGIOutput1: {:?}", e);
                        output_idx += 1;
                        continue;
                    }
                };

                let desc = output1.GetDesc().unwrap_or_else(|e| {
                    warn!("获取输出描述失败: {:?}", e);
                    windows::Win32::Graphics::Dxgi::DXGI_OUTPUT_DESC::default()
                });

                // 尝试创建 Desktop Duplication
                match output1.DuplicateOutput(device) {
                    Ok(dup) => {
                        info!(
                            "已连接到显示器 {} ({}, {}x{})",
                            adapter_idx,
                            output_idx,
                            desc.DesktopCoordinates.right - desc.DesktopCoordinates.left,
                            desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top
                        );
                        let offset_x = desc.DesktopCoordinates.left;
                        let offset_y = desc.DesktopCoordinates.top;
                        return Ok((dup, desc, offset_x, offset_y));
                    }
                    Err(e) => {
                        warn!(
                            "创建 Desktop Duplication 失败 (适配器 {}, 输出 {}): {:?}",
                            adapter_idx, output_idx, e
                        );
                    }
                }

                output_idx += 1;
            }

            adapter_idx += 1;
        }
    }

    /// 创建或更新 staging texture
    unsafe fn ensure_staging_texture(&mut self, width: u32, height: u32, format: DXGI_FORMAT) -> Result<()> {
        // 检查是否需要重新创建
        let need_create = match &self.staging_texture {
            None => true,
            Some(texture) => {
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                desc.Width != width || desc.Height != height || desc.Format != format
            }
        };

        if need_create {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0, // staging texture 不需要绑定
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut texture: Option<ID3D11Texture2D> = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .context("创建 staging texture 失败")?;

            self.staging_texture = Some(texture.context("staging texture 为空")?);
            info!("staging texture 已创建: {}x{}, format={}", width, height, format.0);
        }

        Ok(())
    }

    /// 重新初始化 duplication（分辨率变化时调用）
    fn reinitialize_duplication(&mut self) -> Result<()> {
        unsafe {
            // 获取 factory
            let dxgi_device: IDXGIDevice = self
                .device
                .cast()
                .context("无法将 ID3D11Device 转换为 IDXGIDevice")?;
            let adapter: IDXGIAdapter1 = dxgi_device
                .GetParent()
                .context("无法获取 DXGI Adapter")?;
            let factory: IDXGIFactory1 = adapter
                .GetParent()
                .context("无法获取 DXGI Factory")?;

            // 释放旧的 duplication
            if let Some(dup) = self.duplication.take() {
                let _ = dup.ReleaseFrame();
                drop(dup);
            }

            // 重新创建 duplication
            let (duplication, output_desc, offset_x, offset_y) =
                Self::create_duplication(&factory, &self.device)?;

            self.duplication = Some(duplication);
            self._offset_x = offset_x;
            self._offset_y = offset_y;

            // 更新屏幕信息
            self.info.width = (output_desc.DesktopCoordinates.right - output_desc.DesktopCoordinates.left) as u32;
            self.info.height = (output_desc.DesktopCoordinates.bottom - output_desc.DesktopCoordinates.top) as u32;

            // 清除缓存
            self.staging_texture = None;
            self.last_frame = None;

            info!("Desktop Duplication 已重新初始化: {}x{}", self.info.width, self.info.height);
        }

        Ok(())
    }

    /// 从纹理拷贝像素到 Vec<u8>
    unsafe fn copy_texture_to_buffer(
        &self,
        texture: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        let staging = self
            .staging_texture
            .as_ref()
            .context("staging texture 未初始化")?;

        // 拷贝到 staging texture
        let staging_resource: ID3D11Resource = staging.cast().context("无法转换 staging texture 为 ID3D11Resource")?;
        let texture_resource: ID3D11Resource = texture.cast().context("无法转换 texture 为 ID3D11Resource")?;
        self.context.CopyResource(&staging_resource, &texture_resource);

        // Map staging texture
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        self.context
            .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .context("Map staging texture 失败")?;

        let src_ptr = mapped.pData as *const u8;
        let row_pitch = mapped.RowPitch as usize;
        let dst_row_bytes = (width * 4) as usize;

        // 分配输出缓冲区
        let mut data = vec![0u8; (width * height * 4) as usize];

        // 逐行拷贝，处理 row pitch padding
        for row in 0..height as usize {
            let src_offset = row * row_pitch;
            let dst_offset = row * dst_row_bytes;

            std::ptr::copy_nonoverlapping(
                src_ptr.add(src_offset),
                data.as_mut_ptr().add(dst_offset),
                dst_row_bytes,
            );
        }

        // Unmap
        self.context.Unmap(staging, 0);

        Ok(data)
    }
}

impl ScreenCapturer for WindowsCapturer {
    fn info(&self) -> ScreenInfo {
        self.info.clone()
    }

    fn grab_frame(&mut self) -> Result<Frame> {
        // 先处理 AcquireNextFrame
        let (frame_info, resource) = unsafe {
            let duplication = self.duplication.as_ref().context("duplication 未初始化")?;

            // AcquireNextFrame，超时 500ms
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            let acquire_result = duplication.AcquireNextFrame(500, &mut frame_info, &mut resource);

            match acquire_result {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    // 超时，无新帧
                    if let Some(ref last_frame) = self.last_frame {
                        // 返回缓存的上一帧
                        return Ok(Frame {
                            width: last_frame.width,
                            height: last_frame.height,
                            data: last_frame.data.clone(),
                        });
                    } else {
                        // 没有缓存帧，返回错误
                        bail!("无新帧且无缓存帧");
                    }
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    // 分辨率变化或显示器断开，需要重新初始化
                    warn!("检测到 DXGI_ERROR_ACCESS_LOST，尝试重新初始化");
                    self.reinitialize_duplication()?;
                    bail!("分辨率变化，已重新初始化");
                }
                Err(e) => {
                    bail!("AcquireNextFrame 失败: {:?}", e);
                }
            }

            // 检查是否有累积帧
            if frame_info.AccumulatedFrames == 0 {
                let _ = duplication.ReleaseFrame();
                if let Some(ref last_frame) = self.last_frame {
                    return Ok(Frame {
                        width: last_frame.width,
                        height: last_frame.height,
                        data: last_frame.data.clone(),
                    });
                } else {
                    bail!("无累积帧且无缓存帧");
                }
            }

            (frame_info, resource)
        };

        // 处理光标形状
        unsafe {
            let duplication = self.duplication.as_ref().context("duplication 未初始化")?;
            if frame_info.PointerShapeBufferSize > 0 {
                // 光标形状变化
                let mut buffer = vec![0u8; frame_info.PointerShapeBufferSize as usize];
                let mut shape_info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
                let mut size_written: u32 = 0;

                match duplication.GetFramePointerShape(
                    buffer.len() as u32,
                    buffer.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut size_written as *mut u32,
                    &mut shape_info as *mut DXGI_OUTDUPL_POINTER_SHAPE_INFO,
                ) {
                    Ok(()) => {
                        // 只处理 COLOR 类型 (Type = 1)
                        if shape_info.Type == 1 {
                            // DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR
                            let cursor_width = shape_info.Width;
                            let cursor_height = shape_info.Height;
                            let pitch = shape_info.Pitch as usize;

                            // 转换为 RGBA 格式（BGRA -> RGBA）
                            let mut rgba = Vec::with_capacity((cursor_width * cursor_height * 4) as usize);
                            for row in 0..cursor_height as usize {
                                let row_offset = row * pitch;
                                for col in 0..cursor_width as usize {
                                    let idx = row_offset + col * 4;
                                    if idx + 3 < buffer.len() {
                                        let b = buffer[idx];
                                        let g = buffer[idx + 1];
                                        let r = buffer[idx + 2];
                                        let a = buffer[idx + 3];
                                        rgba.push(r);
                                        rgba.push(g);
                                        rgba.push(b);
                                        rgba.push(a);
                                    }
                                }
                            }

                            self.cursor_cache = Some(CursorShape {
                                width: cursor_width as u16,
                                height: cursor_height as u16,
                                xhot: shape_info.HotSpot.x as u16,
                                yhot: shape_info.HotSpot.y as u16,
                                rgba,
                                serial: self.cursor_serial,
                            });
                            self.cursor_serial = self.cursor_serial.wrapping_add(1);
                        }
                    }
                    Err(e) => {
                        warn!("获取光标形状失败: {:?}", e);
                    }
                }
            }
        }

        // 获取帧纹理并拷贝数据
        unsafe {
            // 获取帧纹理信息（不需要可变借用 self）
            let resource = resource.context("帧资源为空")?;
            let texture: ID3D11Texture2D = resource
                .cast()
                .context("无法将帧资源转换为 ID3D11Texture2D")?;

            // 获取纹理描述
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let width = desc.Width;
            let height = desc.Height;
            let format = desc.Format;

            // 检查格式
            if format != DXGI_FORMAT_B8G8R8A8_UNORM {
                warn!("非 BGRA 格式: {}, 可能需要格式转换", format.0);
            }

            // 确保 staging texture 存在（需要可变借用 self）
            self.ensure_staging_texture(width, height, format)?;

            // 拷贝像素数据
            let data = self.copy_texture_to_buffer(&texture, width, height)?;

            // 释放帧（重新获取 duplication 引用）
            let duplication = self.duplication.as_ref().context("duplication 未初始化")?;
            let _ = duplication.ReleaseFrame();

            // 缓存帧
            let frame = Frame {
                width,
                height,
                data: data.clone(),
            };
            self.last_frame = Some(Frame {
                width,
                height,
                data,
            });

            Ok(frame)
        }
    }

    fn refresh_info(&mut self) -> Result<bool> {
        unsafe {
            // 尝试获取当前帧来检测分辨率变化
            // 如果分辨率变化，AcquireNextFrame 会返回 DXGI_ERROR_ACCESS_LOST
            let duplication = self.duplication.as_ref().context("duplication 未初始化")?;

            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            let result = duplication.AcquireNextFrame(0, &mut frame_info, &mut resource);

            // 释放帧（如果有）
            if result.is_ok() {
                let _ = duplication.ReleaseFrame();
            }

            // 检查是否需要重新初始化
            if let Err(e) = result {
                if e.code() == DXGI_ERROR_ACCESS_LOST {
                    info!("检测到分辨率变化（ACCESS_LOST）");
                    self.reinitialize_duplication()?;
                    return Ok(true);
                }
            }

            Ok(false)
        }
    }

    fn get_cursor_shape(&self) -> Result<Option<CursorShape>> {
        Ok(self.cursor_cache.clone())
    }
}

impl Drop for WindowsCapturer {
    fn drop(&mut self) {
        if let Some(dup) = self.duplication.take() {
            unsafe {
                let _ = dup.ReleaseFrame();
            }
        }
        info!("Windows 屏幕捕获器已释放");
    }
}