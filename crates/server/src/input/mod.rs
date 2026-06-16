//! 输入注入 - 平台抽象 trait + 各平台实现
//! M2 实现

use anyhow::Result;

/// 鼠标按键
#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 键码（与浏览器 KeyboardEvent.code 对应）
#[derive(Debug, Clone, Copy)]
pub struct KeyCode(pub u32);

/// 输入注入器 trait
pub trait InputInjector: Send {
    fn move_mouse(&mut self, x: i32, y: i32) -> Result<()>;
    fn mouse_button(&mut self, btn: MouseButton, pressed: bool) -> Result<()>;
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()>;
    fn key(&mut self, code: KeyCode, pressed: bool) -> Result<()>;
}

/// 工厂函数：创建当前平台的输入注入器
pub fn create_injector() -> Result<Box<dyn InputInjector>> {
    anyhow::bail!("输入注入将在 M2 实现")
}

#[cfg(all(target_os = "linux", feature = "x11"))]
mod x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;
