//! 捕获→编码→分发 线程编排
//! M3 实现：独立线程抓帧+编码，通过 broadcast channel 分发到各连接

use anyhow::{Context, Result};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::capture::{create_capturer, ScreenCapturer, ScreenInfo};
use crate::encoder::{EncodedPacket, H264Encoder};

/// 视频帧二进制协议 magic 标识
const FRAME_MAGIC: u8 = 0xAB;

/// 捕获+编码管线
pub struct CaptureEncoder {
    capturer: Box<dyn ScreenCapturer>,
    encoder: H264Encoder,
}

impl CaptureEncoder {
    /// 创建管线实例
    pub fn new(fps: u32, bitrate: u32) -> Result<(Self, ScreenInfo)> {
        let capturer = create_capturer()
            .context("无法创建屏幕捕获器")?;

        let info = capturer.info();
        info!("屏幕信息: {}x{}", info.width, info.height);

        let encoder = H264Encoder::new(info.width, info.height, fps, bitrate)
            .context("无法创建编码器")?;

        Ok((
            Self { capturer, encoder },
            info,
        ))
    }

    /// 启动捕获+编码线程，返回 (视频帧 broadcast sender, 关键帧请求 sender)
    ///
    /// - `fps`: 目标帧率
    /// - `channel_capacity`: broadcast channel 容量（背压时丢旧帧）
    pub fn start(
        mut self,
        fps: u32,
        channel_capacity: usize,
    ) -> (broadcast::Sender<Vec<u8>>, tokio::sync::mpsc::UnboundedSender<()>) {
        let (video_tx, _) = broadcast::channel(channel_capacity);
        let video_tx_clone = video_tx.clone();
        let (keyframe_tx, keyframe_rx) = tokio::sync::mpsc::unbounded_channel();

        // 独立线程运行捕获+编码循环
        thread::Builder::new()
            .name("capture-encoder".to_string())
            .spawn(move || {
                self.run_loop(fps, video_tx_clone, keyframe_rx);
            })
            .expect("无法启动捕获编码线程");

        info!("捕获编码线程已启动，目标帧率 {}fps", fps);
        (video_tx, keyframe_tx)
    }

    /// 捕获编码主循环（运行在独立线程）
    fn run_loop(
        mut self,
        fps: u32,
        tx: broadcast::Sender<Vec<u8>>,
        mut keyframe_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    ) {
        let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
        let mut frame_count = 0u64;

        loop {
            let loop_start = Instant::now();

            // 0. 检查关键帧请求（非阻塞）
            if keyframe_rx.try_recv().is_ok() {
                self.encoder.request_keyframe();
                debug!("已请求下一帧为关键帧");
            }

            // 1. 抓帧
            let frame = match self.capturer.grab_frame() {
                Ok(f) => f,
                Err(e) => {
                    warn!("抓帧失败: {}", e);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            // 2. 编码
            let packets = match self.encoder.encode(&frame.data) {
                Ok(p) => p,
                Err(e) => {
                    warn!("编码失败: {}", e);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            // 3. 序列化并发送所有数据包
            for packet in packets {
                let binary_frame = serialize_packet(&packet);

                // 发送到 broadcast channel（如果没有接收者会丢弃，无影响）
                let receiver_count = tx.receiver_count();
                if receiver_count > 0 {
                    if let Err(e) = tx.send(binary_frame) {
                        debug!("发送视频帧失败（可能所有接收者都掉线了）: {}", e);
                    }
                }
            }

            frame_count += 1;
            if frame_count % (fps as u64 * 10) == 0 {
                debug!("已编码 {} 帧", frame_count);
            }

            // 4. 节流：按目标帧率间隔
            let elapsed = loop_start.elapsed();
            if elapsed < frame_interval {
                thread::sleep(frame_interval - elapsed);
            } else {
                debug!("捕获+编码耗时 {:?} 超过目标间隔 {:?}", elapsed, frame_interval);
            }
        }
    }
}

/// 序列化编码包为二进制帧格式
///
/// 帧格式（小端序）：
/// ```
/// [magic:u8, frame_type:u8, reserved:u16, sequence:u32, timestamp_us:u64, payload...]
/// ```
fn serialize_packet(packet: &EncodedPacket) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + packet.data.len());

    // 16 字节帧头
    buf.push(FRAME_MAGIC);
    buf.push(packet.frame_type);
    buf.extend_from_slice(&0u16.to_le_bytes());  // reserved
    buf.extend_from_slice(&packet.sequence.to_le_bytes());
    buf.extend_from_slice(&packet.timestamp_us.to_le_bytes());

    // H.264 负载
    buf.extend_from_slice(&packet.data);

    buf
}
