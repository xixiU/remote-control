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
}

/// 工厂函数：创建当前平台的捕获器实现
pub fn create_capturer() -> Result<Box<dyn ScreenCapturer>> {
    anyhow::bail!("屏幕捕获将在 M3 实现")
}

#[cfg(all(target_os = "linux", feature = "x11"))]
mod x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;
