//! X11 屏幕捕获实现（基于 x11rb + MIT-SHM 共享内存扩展，零拷贝高性能）

use super::{Frame, ScreenCapturer, ScreenInfo};
use anyhow::{Context, Result};
use tracing::{info, warn};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::shm::{self};
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

/// X11 屏幕捕获器，优先使用 MIT-SHM 扩展（零拷贝），降级到 XGetImage
///
/// 使用 `RustConnection`（x11rb 默认 `connect` 返回的纯 Rust 实现），
/// 它实现了 `Send`，满足 `ScreenCapturer: Send` 约束。
pub struct X11Capturer {
    /// 与 X server 的连接（读取 DISPLAY 环境变量建立）
    conn: RustConnection,
    /// 根窗口，整屏截图的 drawable
    root: Window,
    /// 屏幕信息（宽高）
    info: ScreenInfo,
    /// MIT-SHM 共享内存状态（可选）
    shm_state: Option<ShmState>,
}

/// MIT-SHM 共享内存状态
struct ShmState {
    /// 共享内存段 ID（System V IPC）
    #[allow(dead_code)]
    shm_id: i32,
    /// 共享内存映射地址
    shm_addr: *mut u8,
    /// 共享内存大小（字节）
    shm_size: usize,
    /// X11 共享内存段标识符
    shm_seg: shm::Seg,
}

// SAFETY: ShmState 包含原始指针，但：
// 1. shm_addr 指向的共享内存由 X Server 和当前进程共享，不会被其他 Rust 代码访问
// 2. X11Capturer 在单线程中使用（pipeline.rs 的独立线程），不会跨线程共享指针
// 3. Drop 时会正确清理资源
unsafe impl Send for ShmState {}

impl X11Capturer {
    /// 创建捕获器，捕获默认屏幕的整个根窗口
    pub fn new() -> Result<Self> {
        // 连接 X server（None 表示从 DISPLAY 环境变量读取目标）
        let (conn, screen_num) =
            x11rb::connect(None).context("无法连接到 X server（请检查 DISPLAY 环境变量）")?;

        // 取默认屏幕的根窗口与像素尺寸
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;

        // 尝试初始化 MIT-SHM 扩展
        let shm_state = match Self::init_shm(&conn, width, height) {
            Ok(state) => {
                info!(
                    "X11 屏幕捕获器已初始化 (MIT-SHM 零拷贝模式): {}x{} (screen #{})",
                    width, height, screen_num
                );
                Some(state)
            }
            Err(e) => {
                warn!("MIT-SHM 初始化失败，降级到 XGetImage: {}", e);
                info!(
                    "X11 屏幕捕获器已初始化 (XGetImage 模式): {}x{} (screen #{})",
                    width, height, screen_num
                );
                None
            }
        };

        Ok(Self {
            conn,
            root,
            info: ScreenInfo { width, height },
            shm_state,
        })
    }

    /// 初始化 MIT-SHM 共享内存
    fn init_shm(conn: &RustConnection, width: u32, height: u32) -> Result<ShmState> {
        // 1. 检查 MIT-SHM 扩展是否可用
        conn.extension_information(shm::X11_EXTENSION_NAME)?
            .context("MIT-SHM 扩展不可用")?;

        // 2. 计算共享内存大小（BGRA 格式，4 字节/像素）
        let shm_size = (width * height * 4) as usize;

        // 3. 创建 System V 共享内存段
        let shm_id = unsafe {
            libc::shmget(
                libc::IPC_PRIVATE,
                shm_size,
                libc::IPC_CREAT | 0o600,
            )
        };
        if shm_id < 0 {
            anyhow::bail!("shmget 失败: {}", std::io::Error::last_os_error());
        }

        // 4. 映射共享内存到进程地址空间
        let shm_addr = unsafe { libc::shmat(shm_id, std::ptr::null(), 0) };
        if shm_addr == libc::MAP_FAILED {
            unsafe { libc::shmctl(shm_id, libc::IPC_RMID, std::ptr::null_mut()) };
            anyhow::bail!("shmat 失败: {}", std::io::Error::last_os_error());
        }

        // 5. 标记共享内存段为删除（进程退出时自动释放）
        unsafe {
            libc::shmctl(shm_id, libc::IPC_RMID, std::ptr::null_mut());
        }

        // 6. 告知 X Server 附加到这个共享内存段
        let shm_seg = conn.generate_id()?;
        shm::attach(
            conn,
            shm_seg,
            shm_id as u32,
            false, // read_only = false，X Server 需要写入权限
        )?
        .check()
        .context("XShmAttach 失败")?;

        info!(
            "MIT-SHM 共享内存已初始化: {} bytes ({}x{} BGRA)",
            shm_size, width, height
        );

        Ok(ShmState {
            shm_id,
            shm_addr: shm_addr as *mut u8,
            shm_size,
            shm_seg,
        })
    }
}

impl ScreenCapturer for X11Capturer {
    fn info(&self) -> ScreenInfo {
        self.info.clone()
    }

    fn grab_frame(&mut self) -> Result<Frame> {
        // 根据 MIT-SHM 状态选择抓帧方式
        if self.shm_state.is_some() {
            self.grab_frame_shm()
        } else {
            self.grab_frame_fallback()
        }
    }

    fn refresh_info(&mut self) -> Result<bool> {
        // 重新查询根窗口的几何信息以检测分辨率变化
        let geom = self
            .conn
            .get_geometry(self.root)
            .context("查询根窗口几何信息失败")?
            .reply()
            .context("接收几何信息回复失败")?;

        let new_width = geom.width as u32;
        let new_height = geom.height as u32;

        if new_width != self.info.width || new_height != self.info.height {
            info!(
                "检测到屏幕分辨率变化: {}x{} → {}x{}",
                self.info.width, self.info.height, new_width, new_height
            );

            // 释放旧的 MIT-SHM 资源
            if let Some(old_state) = self.shm_state.take() {
                Self::cleanup_shm(&self.conn, old_state);
            }

            // 更新屏幕信息
            self.info.width = new_width;
            self.info.height = new_height;

            // 尝试重新初始化 MIT-SHM
            match Self::init_shm(&self.conn, new_width, new_height) {
                Ok(state) => {
                    info!("MIT-SHM 已重新初始化为新分辨率");
                    self.shm_state = Some(state);
                }
                Err(e) => {
                    warn!("MIT-SHM 重新初始化失败，降级到 XGetImage: {}", e);
                    self.shm_state = None;
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl X11Capturer {
    /// MIT-SHM 模式抓帧（零拷贝）
    fn grab_frame_shm(&mut self) -> Result<Frame> {
        let shm_state = self.shm_state.as_ref().unwrap();
        let width = self.info.width as u16;
        let height = self.info.height as u16;

        // 使用 MIT-SHM 扩展抓取屏幕到共享内存
        let reply = shm::get_image(
            &self.conn,
            self.root,
            0,
            0,
            width,
            height,
            !0, // plane_mask: 全部位平面
            ImageFormat::Z_PIXMAP.into(), // 转换为 u8
            shm_state.shm_seg,
            0, // offset: 从共享内存起始位置写入
        )?
        .reply()
        .context("XShmGetImage 失败")?;

        // 调试信息（首次抓帧时输出）
        static FIRST_FRAME: std::sync::Once = std::sync::Once::new();
        FIRST_FRAME.call_once(|| {
            info!(
                "MIT-SHM get_image reply: depth={}, visual={}, size={}",
                reply.depth, reply.visual, reply.size
            );
        });

        // 直接从共享内存读取数据
        // 注意：X Server 在 reply 返回后已经写入共享内存，无需额外同步
        let data_size = reply.size as usize;
        let expected_size = (width as usize) * (height as usize) * 4;

        // 校验返回的数据大小
        if data_size != expected_size {
            anyhow::bail!(
                "MIT-SHM 返回数据大小不符: 实际 {} 字节, 期望 {} 字节 (depth={}, {}x{})",
                data_size,
                expected_size,
                reply.depth,
                width,
                height
            );
        }

        let data = unsafe {
            std::slice::from_raw_parts(shm_state.shm_addr, data_size).to_vec()
        };

        // 调试：检查前几个字节（仅首次）
        FIRST_FRAME.call_once(|| {
            if data.len() >= 16 {
                info!(
                    "MIT-SHM data first 16 bytes: {:02X?}",
                    &data[..16]
                );
            }
        });

        Ok(Frame {
            width: self.info.width,
            height: self.info.height,
            data,
        })
    }

    /// XGetImage 降级模式（传统方式）
    fn grab_frame_fallback(&mut self) -> Result<Frame> {
        let width = self.info.width as u16;
        let height = self.info.height as u16;

        // 用 ZPixmap 格式抓取整个根窗口
        let reply = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                width,
                height,
                !0,
            )
            .context("发送 get_image 请求失败")?
            .reply()
            .context("接收 get_image 回复失败")?;

        let data = reply.data;

        // 校验数据长度
        let expected = self.info.width as usize * self.info.height as usize * 4;
        if data.len() != expected {
            anyhow::bail!(
                "get_image 返回数据长度不符: 实际 {} 字节, 期望 {} 字节 (depth={}, {}x{})。\
                 可能是非 32 位 visual，暂不支持",
                data.len(),
                expected,
                reply.depth,
                self.info.width,
                self.info.height
            );
        }

        Ok(Frame {
            width: self.info.width,
            height: self.info.height,
            data,
        })
    }

    /// 清理 MIT-SHM 资源
    fn cleanup_shm(conn: &RustConnection, state: ShmState) {
        // 1. 从 X Server 分离共享内存段
        if let Err(e) = shm::detach(conn, state.shm_seg) {
            warn!("XShmDetach 失败: {:?}", e);
        }

        // 2. 解除进程地址空间映射
        unsafe {
            if libc::shmdt(state.shm_addr as *const libc::c_void) != 0 {
                warn!("shmdt 失败: {}", std::io::Error::last_os_error());
            }
        }

        // 注意：shmctl(IPC_RMID) 已在 init_shm 时调用，内核会在最后一个进程分离后自动删除
    }
}

impl Drop for X11Capturer {
    fn drop(&mut self) {
        if let Some(state) = self.shm_state.take() {
            Self::cleanup_shm(&self.conn, state);
        }
    }
}
