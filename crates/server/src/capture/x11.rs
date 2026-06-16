//! X11 屏幕捕获实现（基于 x11rb，纯 Rust 调用 XGetImage，无额外 C 库依赖）

use super::{Frame, ScreenCapturer, ScreenInfo};
use anyhow::{Context, Result};
use tracing::info;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

/// X11 屏幕捕获器，基于 x11rb 库实现
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
}

impl X11Capturer {
    /// 创建捕获器，捕获默认屏幕的整个根窗口
    pub fn new() -> Result<Self> {
        // 连接 X server（None 表示从 DISPLAY 环境变量读取目标）
        let (conn, screen_num) =
            x11rb::connect(None).context("无法连接到 X server（请检查 DISPLAY 环境变量）")?;

        // 取默认屏幕的根窗口与像素尺寸
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;

        info!(
            "X11 屏幕捕获器已初始化: {}x{} (screen #{})",
            width, height, screen_num
        );

        Ok(Self {
            conn,
            root,
            info: ScreenInfo {
                width: width as u32,
                height: height as u32,
            },
        })
    }
}

impl ScreenCapturer for X11Capturer {
    fn info(&self) -> ScreenInfo {
        self.info.clone()
    }

    fn grab_frame(&mut self) -> Result<Frame> {
        let width = self.info.width as u16;
        let height = self.info.height as u16;

        // 用 ZPixmap 格式抓取整个根窗口。
        // plane_mask 取 !0（全部位平面）。
        //
        // 注意：一次性抓取整屏的数据量可能超过 X11 协议的单请求/单回复上限
        // （受 maximum-request-length 限制，约 256KB 起，启用 BIG-REQUESTS 后更大）。
        // 在 4K 等高分辨率下若出现运行时报错，需改为分块（按行/块多次 get_image）
        // 或使用 MIT-SHM 扩展抓取。这里先实现整屏抓取以保证接口与编译正确。
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

        // 校验数据长度。常见 24/32 位深的 TrueColor visual 下，
        // ZPixmap 每像素占 4 字节，小端 X server 上字节顺序即为 B,G,R,X，
        // 正好与下游期望的 BGRA 一致（X 通道当作 alpha）。
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
}
