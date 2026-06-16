//! X11 屏幕捕获实现（M3 实现）

use super::{Frame, ScreenCapturer, ScreenInfo};
use anyhow::Result;

pub struct X11Capturer;

impl X11Capturer {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl ScreenCapturer for X11Capturer {
    fn info(&self) -> ScreenInfo {
        todo!("M3: 获取 X11 屏幕信息")
    }

    fn grab_frame(&mut self) -> Result<Frame> {
        todo!("M3: X11 抓帧")
    }
}
