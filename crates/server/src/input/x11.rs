//! X11 输入注入实现（基于 enigo 0.6.x + x11rb / XTest）。
//!
//! 一个连接持有一个 `X11Injector`：内部包装 `enigo::Enigo`，
//! 把 trait 抽象的 move/click/scroll/key 调用转成 enigo 调用。

use super::{keymap, InputInjector, MouseButton};
use anyhow::{Context, Result};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use tracing::{debug, warn};

pub struct X11Injector {
    enigo: Enigo,
}

impl X11Injector {
    /// 初始化 enigo 实例。失败一般意味着无法连上 X server（无 DISPLAY 等）。
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .context("初始化 enigo（X11）失败：检查 DISPLAY 与 XTest 扩展")?;
        Ok(Self { enigo })
    }
}

fn dir(pressed: bool) -> Direction {
    if pressed {
        Direction::Press
    } else {
        Direction::Release
    }
}

fn map_button(btn: MouseButton) -> Button {
    match btn {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

/// 把浏览器 wheel 像素 delta 转换为 enigo scroll 步数。
/// 至少 1 步、最多 10 步，避免极端值触发风暴。
fn scroll_steps(delta: i32) -> i32 {
    if delta == 0 {
        return 0;
    }
    let steps = delta / 100;
    if steps == 0 {
        if delta > 0 {
            1
        } else {
            -1
        }
    } else {
        steps.clamp(-10, 10)
    }
}

impl InputInjector for X11Injector {
    fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
        self.enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| anyhow::anyhow!("move_mouse 失败: {e:?}"))
    }

    fn mouse_button(&mut self, btn: MouseButton, pressed: bool) -> Result<()> {
        self.enigo
            .button(map_button(btn), dir(pressed))
            .map_err(|e| anyhow::anyhow!("mouse_button 失败: {e:?}"))
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()> {
        // 浏览器 wheel 的 dy 通常以 100~120 为单位，方向：dy>0 向下、dy<0 向上。
        // enigo Vertical 的语义：正值向下、负值向上 —— 与浏览器一致。
        // 这里把像素级 delta 简化为离散步数（每 100px 一步、至少 1 步），
        // 防止把巨大的 length 直接喂给 XTest 触发风暴式滚轮事件。
        let steps_y = scroll_steps(dy);
        let steps_x = scroll_steps(dx);

        if steps_y != 0 {
            self.enigo
                .scroll(steps_y, Axis::Vertical)
                .map_err(|e| anyhow::anyhow!("scroll(vertical) 失败: {e:?}"))?;
        }
        if steps_x != 0 {
            self.enigo
                .scroll(steps_x, Axis::Horizontal)
                .map_err(|e| anyhow::anyhow!("scroll(horizontal) 失败: {e:?}"))?;
        }
        Ok(())
    }

    fn key(&mut self, code: &str, pressed: bool) -> Result<()> {
        let Some(key) = keymap::code_to_key(code) else {
            warn!("未支持的键码 '{}'，已忽略", code);
            return Ok(());
        };
        debug!("inject key code='{}' pressed={}", code, pressed);
        self.enigo
            .key(key, dir(pressed))
            .map_err(|e| anyhow::anyhow!("key('{code}') 失败: {e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::scroll_steps;

    #[test]
    fn scroll_steps_zero_in_zero_out() {
        assert_eq!(scroll_steps(0), 0);
    }

    #[test]
    fn scroll_steps_small_delta_at_least_one_step() {
        assert_eq!(scroll_steps(40), 1);
        assert_eq!(scroll_steps(-50), -1);
    }

    #[test]
    fn scroll_steps_large_delta_uses_floor_div() {
        assert_eq!(scroll_steps(120), 1);
        assert_eq!(scroll_steps(250), 2);
        assert_eq!(scroll_steps(-300), -3);
    }

    #[test]
    fn scroll_steps_clamps_to_ten() {
        assert_eq!(scroll_steps(100_000), 10);
        assert_eq!(scroll_steps(-100_000), -10);
    }
}
