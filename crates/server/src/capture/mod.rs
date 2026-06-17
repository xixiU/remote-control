//! 屏幕捕获 - 平台抽象 trait + 各平台实现
//! M3 实现

use anyhow::Result;

/// 屏幕信息
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
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
}

/// 工厂函数：创建当前平台的捕获器实现
pub fn create_capturer() -> Result<Box<dyn ScreenCapturer>> {
    #[cfg(all(target_os = "linux", feature = "x11"))]
    {
        let capturer = x11::X11Capturer::new()?;
        return Ok(Box::new(capturer));
    }

    #[cfg(target_os = "windows")]
    {
        anyhow::bail!("Windows 屏幕捕获将在后续里程碑实现")
    }

    #[cfg(all(target_os = "linux", feature = "wayland"))]
    {
        anyhow::bail!("Wayland 屏幕捕获将在后续里程碑实现")
    }

    #[cfg(not(any(
        all(target_os = "linux", feature = "x11"),
        target_os = "windows",
        all(target_os = "linux", feature = "wayland")
    )))]
    {
        anyhow::bail!("当前平台不支持屏幕捕获")
    }
}

#[cfg(all(target_os = "linux", feature = "x11"))]
mod x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;
