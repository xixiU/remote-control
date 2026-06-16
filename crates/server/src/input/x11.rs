//! X11 输入注入实现（M2 实现）

use super::{InputInjector, KeyCode, MouseButton};
use anyhow::Result;

pub struct X11Injector;

impl X11Injector {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl InputInjector for X11Injector {
    fn move_mouse(&mut self, _x: i32, _y: i32) -> Result<()> {
        todo!("M2: X11 鼠标移动")
    }

    fn mouse_button(&mut self, _btn: MouseButton, _pressed: bool) -> Result<()> {
        todo!("M2: X11 鼠标按键")
    }

    fn scroll(&mut self, _dx: i32, _dy: i32) -> Result<()> {
        todo!("M2: X11 滚轮")
    }

    fn key(&mut self, _code: KeyCode, _pressed: bool) -> Result<()> {
        todo!("M2: X11 键盘注入")
    }
}
