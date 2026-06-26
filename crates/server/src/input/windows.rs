//! Windows 输入注入实现（enigo 后端）
//!
//! 基于 enigo crate 实现鼠标和键盘输入注入。

use super::{InputInjector, MouseButton, keymap};
use anyhow::Result;
use enigo::{Button, Coordinate, Direction, Enigo, Keyboard, Mouse};

/// Windows 输入注入器
pub struct WindowsInjector {
    enigo: Enigo,
}

impl WindowsInjector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            enigo: Enigo::new(&enigo::Settings::default())?,
        })
    }
}

impl InputInjector for WindowsInjector {
    fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
        self.enigo.move_mouse(x, y, Coordinate::Abs)?;
        Ok(())
    }

    fn mouse_button(&mut self, btn: MouseButton, pressed: bool) -> Result<()> {
        let enigo_btn = match btn {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
        };

        let direction = if pressed {
            Direction::Press
        } else {
            Direction::Release
        };

        self.enigo.button(enigo_btn, direction)?;
        Ok(())
    }

    fn scroll(&mut self, _dx: i32, dy: i32) -> Result<()> {
        // enigo 的 scroll 方法单位是"点击"
        // 简单映射：正数向上滚动，负数向下滚动
        if dy > 0 {
            self.enigo.button(Button::ScrollUp, Direction::Press)?;
            self.enigo.button(Button::ScrollUp, Direction::Release)?;
        } else if dy < 0 {
            self.enigo.button(Button::ScrollDown, Direction::Press)?;
            self.enigo.button(Button::ScrollDown, Direction::Release)?;
        }
        // 横向滚轮暂不支持（enigo 限制）
        Ok(())
    }

    fn key(&mut self, code: &str, pressed: bool) -> Result<()> {
        let key = keymap::code_to_key(code);
        if let Some(key) = key {
            let direction = if pressed {
                Direction::Press
            } else {
                Direction::Release
            };
            self.enigo.key(key, direction)?;
        }
        Ok(())
    }
}