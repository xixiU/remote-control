//! 输入注入 - 平台抽象 trait + 各平台实现
//! M2 实现：定义 trait 与工厂；x11 子模块为 Linux+x11 提供 enigo 实现。

use anyhow::Result;

pub mod keymap;

/// 鼠标按键
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    /// 从协议字符串（"left"/"right"/"middle"）解析。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            _ => None,
        }
    }
}

/// 输入注入器 trait
///
/// 实现需要 `Send`，便于跨线程使用（一个会话/连接持有一个注入器）。
pub trait InputInjector: Send {
    /// 移动鼠标到屏幕绝对像素坐标。
    fn move_mouse(&mut self, x: i32, y: i32) -> Result<()>;

    /// 鼠标按键按下/释放。
    fn mouse_button(&mut self, btn: MouseButton, pressed: bool) -> Result<()>;

    /// 滚轮：dx 横向，dy 纵向。单位是浏览器 wheel 事件的 deltaX/deltaY。
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()>;

    /// 键盘按键按下/释放。`code` 取自浏览器 `KeyboardEvent.code`。
    /// 实现内部应通过 [`keymap::code_to_key`] 映射；映射不到时记录日志并返回 Ok。
    fn key(&mut self, code: &str, pressed: bool) -> Result<()>;
}

/// 工厂函数：创建当前平台的输入注入器。
///
/// - Linux + feature `x11`：返回 `X11Injector`（基于 enigo + x11rb / XTest）。
/// - Windows：返回 `WindowsInjector`（基于 enigo + SendInput）。
/// - 其它平台：暂未实现，返回错误。
pub fn create_injector() -> Result<Box<dyn InputInjector>> {
    #[cfg(target_os = "windows")]
    {
        let injector = windows::WindowsInjector::new()?;
        return Ok(Box::new(injector));
    }

    #[cfg(all(target_os = "linux", feature = "x11"))]
    {
        let injector = x11::X11Injector::new()?;
        return Ok(Box::new(injector));
    }

    #[cfg(not(any(target_os = "windows", all(target_os = "linux", feature = "x11"))))]
    {
        anyhow::bail!("当前平台尚未实现输入注入");
    }
}

#[cfg(all(target_os = "linux", feature = "x11"))]
mod x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_parses_protocol_strings() {
        assert_eq!(MouseButton::from_str("left"), Some(MouseButton::Left));
        assert_eq!(MouseButton::from_str("right"), Some(MouseButton::Right));
        assert_eq!(MouseButton::from_str("middle"), Some(MouseButton::Middle));
        assert_eq!(MouseButton::from_str("Left"), None);
        assert_eq!(MouseButton::from_str(""), None);
    }
}
