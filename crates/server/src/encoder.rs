//! 视频编码（捕获帧 → H.264 码流）
//! M3 实现：使用 openh264 软件编码，低延迟参数配置

use anyhow::{Context, Result};
use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::YUVSlices;
use openh264::OpenH264API;
use tracing::{debug, info};

/// 编码后的数据包
#[derive(Debug, Clone)]
pub struct EncodedPacket {
    /// 帧类型：1=IDR关键帧, 2=P帧, 3=参数集(SPS/PPS)
    pub frame_type: u8,
    /// 帧序号（单调递增）
    pub sequence: u32,
    /// 捕获时间戳（微秒）
    pub timestamp_us: u64,
    /// H.264 Annex-B 格式的负载数据
    pub data: Vec<u8>,
}

/// H.264 编码器
pub struct H264Encoder {
    encoder: Encoder,
    width: u32,
    height: u32,
    sequence: u32,
}

impl H264Encoder {
    /// 创建编码器
    ///
    /// - `width`, `height`: 视频分辨率
    /// - `fps`: 目标帧率
    /// - `bitrate`: 目标码率（bps）。传入 `<= 0`（约定 `-1`）时按分辨率与帧率
    ///   自动估算一个适合桌面内容的码率。
    pub fn new(width: u32, height: u32, fps: u32, bitrate: i32) -> Result<Self> {
        // 码率 <= 0 表示自动：按分辨率和帧率估算。
        let bitrate = if bitrate <= 0 {
            let auto = auto_bitrate(width, height, fps);
            info!(
                "码率自动估算: {} bps ({}x{} @ {}fps)",
                auto, width, height, fps
            );
            auto
        } else {
            bitrate as u32
        };

        // openh264 0.6 的 EncoderConfig::new() 不接收分辨率参数，
        // 分辨率在 encode() 时由输入 YUV 尺寸决定。
        let config = EncoderConfig::new()
            .max_frame_rate(fps as f32)
            .set_bitrate_bps(bitrate)
            .enable_skip_frame(false);

        let api = OpenH264API::from_source();
        let encoder = Encoder::with_api_config(api, config)
            .context("无法初始化 openh264 编码器")?;

        info!(
            "H.264 编码器已初始化: {}x{} @ {}fps, {}bps",
            width, height, fps, bitrate
        );

        Ok(Self {
            encoder,
            width,
            height,
            sequence: 0,
        })
    }

    /// 编码一帧 BGRA 数据
    ///
    /// 返回编码后的数据包列表（一帧可能产生多个包，如 SPS/PPS + IDR）
    pub fn encode(&mut self, bgra_data: &[u8]) -> Result<Vec<EncodedPacket>> {
        let w = self.width as usize;
        let h = self.height as usize;

        // BGRA → YUV420 转换，得到三个平面
        let (y_plane, u_plane, v_plane) = bgra_to_yuv420(bgra_data, self.width, self.height)?;

        // 构造 YUVSlices（借用平面数据）
        let yuv = YUVSlices::new(
            (&y_plane, &u_plane, &v_plane),
            (w, h),
            (w, w / 2, w / 2),
        );

        // 编码
        let timestamp_us = timestamp_micros();
        let bitstream = self.encoder
            .encode(&yuv)
            .context("openh264 编码失败")?;

        // 解析 NAL 单元。openh264 0.6 用 num_layers()/layer(i) 访问各层。
        let mut packets = Vec::new();

        for i in 0..bitstream.num_layers() {
            let layer = match bitstream.layer(i) {
                Some(l) => l,
                None => continue,
            };
            for n in 0..layer.nal_count() {
                let nal = match layer.nal_unit(n) {
                    Some(s) => s,
                    None => continue,
                };
                let nal_data = nal.to_vec();

                // 根据 NAL 类型判断帧类型
                // 注意：openh264 输出的 NAL 可能包含 Annex-B 起始码（00 00 00 01）。
                // 需要跳过起始码找到真正的 NAL 头字节。
                let nal_header_offset = if nal_data.len() >= 4
                    && nal_data[0] == 0
                    && nal_data[1] == 0
                    && nal_data[2] == 0
                    && nal_data[3] == 1
                {
                    4
                } else if nal_data.len() >= 3 && nal_data[0] == 0 && nal_data[1] == 0 && nal_data[2] == 1
                {
                    3
                } else {
                    0
                };

                // NAL 类型在 NAL 头字节的低 5 位
                let nal_type = nal_data
                    .get(nal_header_offset)
                    .map(|&b| b & 0x1F)
                    .unwrap_or(0);

                let frame_type = match nal_type {
                    5 => 1,  // IDR 关键帧
                    1 => 2,  // P 帧
                    7 | 8 => 3,  // SPS(7) / PPS(8) 参数集
                    _ => {
                        debug!("未知 NAL 类型: {} (前8字节: {:02X?})", nal_type, &nal_data[..8.min(nal_data.len())]);
                        continue;
                    }
                };

                self.sequence += 1;
                packets.push(EncodedPacket {
                    frame_type,
                    sequence: self.sequence,
                    timestamp_us,
                    data: nal_data,
                });
            }
        }

        Ok(packets)
    }

    /// 请求下一帧强制编码为关键帧（IDR）
    pub fn request_keyframe(&mut self) {
        self.encoder.force_intra_frame();
    }
}

/// 根据分辨率和帧率自动估算适合桌面内容的码率（bps）
///
/// 思路：以「每像素比特数 (bpp)」为基准，再按帧率相对 30fps 缩放。
/// - 桌面内容（文字、锐利边缘）比视频更吃码率，取 bpp ≈ 0.10 作为基准，
///   使 1080p@30fps 约为 6.2 Mbps，叠加下方下限/上限后落在清晰区间。
/// - 低帧率下每帧分到的码率更多，按 sqrt(fps/30) 缩放避免线性下降过猛，
///   保证 10fps 这类低帧率也维持足够清晰度。
/// - 最终钳制在 [2 Mbps, 25 Mbps]，避免极端分辨率给出离谱值。
fn auto_bitrate(width: u32, height: u32, fps: u32) -> u32 {
    const BITS_PER_PIXEL: f64 = 0.10;
    const MIN_BITRATE: f64 = 2_000_000.0;
    const MAX_BITRATE: f64 = 25_000_000.0;

    let pixels = (width as f64) * (height as f64);
    let fps = fps.max(1) as f64;
    let fps_factor = (fps / 30.0).sqrt();

    let raw = pixels * 30.0 * BITS_PER_PIXEL * fps_factor;
    raw.clamp(MIN_BITRATE, MAX_BITRATE) as u32
}

/// BGRA → YUV420 颜色转换（简单实现，性能优化留给 M5）
///
/// 返回 (Y平面, U平面, V平面) 三个 owned buffer。
fn bgra_to_yuv420(
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let w = width as usize;
    let h = height as usize;
    let expected_len = w * h * 4;

    anyhow::ensure!(
        bgra.len() == expected_len,
        "BGRA 数据长度不匹配: 预期 {} 字节, 实际 {} 字节",
        expected_len,
        bgra.len()
    );

    // Y 平面 (亮度): 全分辨率
    let mut y_plane = vec![0u8; w * h];
    // U/V 平面 (色度): 1/4 分辨率 (2x2 子采样)
    let uv_size = (w / 2) * (h / 2);
    let mut u_plane = vec![0u8; uv_size];
    let mut v_plane = vec![0u8; uv_size];

    // 转换每个像素
    for row in 0..h {
        for col in 0..w {
            let idx = (row * w + col) * 4;
            let b = bgra[idx] as f32;
            let g = bgra[idx + 1] as f32;
            let r = bgra[idx + 2] as f32;
            // bgra[idx + 3] 是 alpha，不需要

            // BT.601 颜色空间转换公式
            let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
            y_plane[row * w + col] = y;

            // 色度平面：每 2x2 块采样一次
            if row % 2 == 0 && col % 2 == 0 {
                let u = ((-0.169 * r - 0.331 * g + 0.500 * b) + 128.0).clamp(0.0, 255.0) as u8;
                let v = ((0.500 * r - 0.419 * g - 0.081 * b) + 128.0).clamp(0.0, 255.0) as u8;
                let uv_idx = (row / 2) * (w / 2) + (col / 2);
                u_plane[uv_idx] = u;
                v_plane[uv_idx] = v;
            }
        }
    }

    Ok((y_plane, u_plane, v_plane))
}

/// 获取当前时间戳（微秒）
fn timestamp_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}
