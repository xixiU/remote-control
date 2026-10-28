//! 屏幕捕获 - 平台抽象 trait + 各平台实现
//! M3 实现

use anyhow::Result;

/// 屏幕信息
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
}

/// 光标形状信息
#[derive(Debug, Clone, PartialEq)]
pub struct CursorShape {
    pub width: u16,
    pub height: u16,
    pub xhot: u16,
    pub yhot: u16,
    /// RGBA 像素数据（未预乘 alpha）
    pub rgba: Vec<u8>,
    /// 光标序列号（形状变化时递增）
    pub serial: u32,
}

/// 捕获的一帧画面
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// BGRA 像素数据
    pub data: Vec<u8>,
}

/// 屏幕捕获器 trait
pub trait ScreenCapturer: Send {
    /// 返回屏幕分辨率等信息
    fn info(&self) -> ScreenInfo;
    /// 抓取一帧，返回原始像素（BGRA）
    fn grab_frame(&mut self) -> Result<Frame>;
    /// 检测并更新屏幕分辨率。返回 Ok(true) 表示分辨率已变化。
    fn refresh_info(&mut self) -> Result<bool>;
    /// 获取当前光标形状。返回 None 表示无光标或不支持。
    fn get_cursor_shape(&self) -> Result<Option<CursorShape>>;
}

/// 工厂函数：创建当前平台的捕获器实现
///
/// Linux 下的选择逻辑：
/// - 同时启用 `wayland` + `x11` feature 时，通过 `WAYLAND_DISPLAY` 环境变量
///   判断当前是否为 Wayland 会话，是则走 `WaylandCapturer`，否则走 `X11Capturer`。
/// - 只启用 `wayland`：一律走 Wayland 后端（内部仍依赖 Xwayland 桥接）。
/// - 只启用 `x11`：走原 `X11Capturer`。
pub fn create_capturer() -> Result<Box<dyn ScreenCapturer>> {
    #[cfg(target_os = "windows")]
    {
        let capturer = windows::WindowsCapturer::new()?;
        return Ok(Box::new(capturer));
    }

    #[cfg(all(target_os = "linux", feature = "wayland"))]
    {
        // Wayland 会话优先走 Wayland 后端；纯 X11 会话降级到 X11Capturer。
        if is_wayland_session() {
            let capturer = wayland::WaylandCapturer::new()?;
            return Ok(Box::new(capturer));
        }
    }

    #[cfg(all(target_os = "linux", any(feature = "x11", feature = "wayland")))]
    {
        let capturer = x11::X11Capturer::new()?;
        return Ok(Box::new(capturer));
    }

    #[cfg(not(any(
        target_os = "windows",
        all(target_os = "linux", any(feature = "x11", feature = "wayland")),
    )))]
    {
        anyhow::bail!("当前平台不支持屏幕捕获")
    }
}

/// 判断当前是否运行在 Wayland 会话中。
///
/// 判定顺序：`WAYLAND_DISPLAY` 非空 > `XDG_SESSION_TYPE == "wayland"`。
#[cfg(all(target_os = "linux", feature = "wayland"))]
fn is_wayland_session() -> bool {
    if std::env::var("WAYLAND_DISPLAY").ok().filter(|v| !v.is_empty()).is_some() {
        return true;
    }
    matches!(std::env::var("XDG_SESSION_TYPE").as_deref(), Ok("wayland"))
}

#[cfg(all(target_os = "linux", any(feature = "x11", feature = "wayland")))]
mod x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;
