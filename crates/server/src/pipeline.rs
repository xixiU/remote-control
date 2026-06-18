//! 捕获→编码→分发 线程编排
//! M3 实现：独立线程抓帧+编码，通过 broadcast channel 分发到各连接

use anyhow::{Context, Result};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::capture::{create_capturer, ScreenCapturer, ScreenInfo};
use crate::encoder::{EncodedPacket, H264Encoder};
use tokio::sync::mpsc;

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

    /// 启动捕获+编码线程，返回 (视频帧 broadcast sender, 关键帧请求 sender, 屏幕信息变化 receiver)
    ///
    /// - `fps`: 目标帧率
    /// - `bitrate`: 目标码率（用于分辨率变化时重新初始化编码器）
    /// - `channel_capacity`: broadcast channel 容量（背压时丢旧帧）
    pub fn start(
        self,
        fps: u32,
        bitrate: u32,
        channel_capacity: usize,
    ) -> (
        broadcast::Sender<Vec<u8>>,
        tokio::sync::mpsc::UnboundedSender<()>,
        mpsc::UnboundedReceiver<ScreenInfo>,
    ) {
        let (video_tx, _) = broadcast::channel(channel_capacity);
        let video_tx_clone = video_tx.clone();
        let (keyframe_tx, keyframe_rx) = tokio::sync::mpsc::unbounded_channel();
        let (screen_info_tx, screen_info_rx) = mpsc::unbounded_channel();

        // 独立线程运行捕获+编码循环
        thread::Builder::new()
            .name("capture-encoder".to_string())
            .spawn(move || {
                self.run_loop(fps, bitrate, video_tx_clone, keyframe_rx, screen_info_tx);
            })
            .expect("无法启动捕获编码线程");

        info!("捕获编码线程已启动，目标帧率 {}fps", fps);
        (video_tx, keyframe_tx, screen_info_rx)
    }

    /// 捕获编码主循环（运行在独立线程）
    fn run_loop(
        mut self,
        fps: u32,
        bitrate: u32,
        tx: broadcast::Sender<Vec<u8>>,
        mut keyframe_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
        screen_info_tx: mpsc::UnboundedSender<ScreenInfo>,
    ) {
        let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
        // 无订阅者时的轮询间隔：仅用于探测是否有新连接，不抓帧不编码。
        let idle_poll = Duration::from_millis(100);
        // 分辨率检测间隔：每秒检查一次（避免每帧都查询 X11 服务器）
        let resolution_check_interval = Duration::from_secs(1);
        let mut last_resolution_check = Instant::now();
        let mut frame_count = 0u64;
        // 是否处于空闲（无订阅者）状态。初始为 true：启动后无连接即不工作。
        let mut idle = true;

        loop {
            let loop_start = Instant::now();

            // 没有任何订阅者时进入空闲：跳过抓帧与编码（编码里的逐像素
            // BGRA→YUV 转换是主要 CPU 开销），只按 idle_poll 间隔探测新连接。
            if tx.receiver_count() == 0 {
                if !idle {
                    idle = true;
                    debug!("无订阅者，捕获编码进入空闲");
                }
                // 清掉空闲期间可能堆积的关键帧请求；恢复时会无条件强制一次。
                while keyframe_rx.try_recv().is_ok() {}
                thread::sleep(idle_poll);
                continue;
            }

            // 从空闲恢复：强制下一帧为关键帧，确保新订阅者能拿到 SPS/PPS/IDR。
            if idle {
                idle = false;
                self.encoder.request_keyframe();
                debug!("检测到订阅者，恢复捕获并强制关键帧");
            }

            // 0. 定期检查屏幕分辨率是否变化
            if loop_start.duration_since(last_resolution_check) >= resolution_check_interval {
                last_resolution_check = loop_start;
                match self.capturer.refresh_info() {
                    Ok(true) => {
                        // 分辨率已变化，重新初始化编码器
                        let new_info = self.capturer.info();
                        info!(
                            "屏幕分辨率已变化为 {}x{}，重新初始化编码器",
                            new_info.width, new_info.height
                        );

                        match H264Encoder::new(new_info.width, new_info.height, fps, bitrate) {
                            Ok(new_encoder) => {
                                self.encoder = new_encoder;
                                // 通知 web 层向所有客户端广播新的屏幕信息
                                if let Err(e) = screen_info_tx.send(new_info.clone()) {
                                    warn!("发送屏幕信息变化通知失败: {}", e);
                                }
                            }
                            Err(e) => {
                                warn!("重新初始化编码器失败: {}，继续使用旧编码器", e);
                            }
                        }
                    }
                    Ok(false) => {
                        // 分辨率未变化，继续
                    }
                    Err(e) => {
                        debug!("检查屏幕分辨率失败: {}", e);
                    }
                }
            }

            // 1. 检查关键帧请求（非阻塞）
            if keyframe_rx.try_recv().is_ok() {
                self.encoder.request_keyframe();
                debug!("已请求下一帧为关键帧");
            }

            // 2. 抓帧
            let frame = match self.capturer.grab_frame() {
                Ok(f) => f,
                Err(e) => {
                    warn!("抓帧失败: {}", e);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            // 3. 编码
            let packets = match self.encoder.encode(&frame.data) {
                Ok(p) => p,
                Err(e) => {
                    warn!("编码失败: {}", e);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            // 4. 序列化并发送所有数据包。能走到这里说明至少有一个订阅者。
            for packet in packets {
                let binary_frame = serialize_packet(&packet);
                if let Err(e) = tx.send(binary_frame) {
                    debug!("发送视频帧失败（可能所有接收者都掉线了）: {}", e);
                }
            }

            frame_count += 1;
            if frame_count % (fps as u64 * 10) == 0 {
                debug!("已编码 {} 帧", frame_count);
            }

            // 5. 节流：按目标帧率间隔
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
