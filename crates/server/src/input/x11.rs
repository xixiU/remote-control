//! X11 输入注入实现（移动/点击/键盘走 enigo；滚轮走自带的 x11rb XTest）。
//!
//! 一个连接持有一个 `X11Injector`。移动/点击/键盘用 `enigo::Enigo`。
//! 滚轮不走 enigo：实测 enigo 的 Button4/5 注入在 UKUI/虚拟机环境下不生效，
//! 而 `xdotool click 4/5`（同样走 XTest）却可用。差异在于 enigo 显式传了它查到的
//! `IS_X_POINTER` 设备 id，这里改为复刻 xdotool 的调用：deviceid 传 0（默认设备）。

use super::{keymap, InputInjector, MouseButton};
use anyhow::{Context, Result};
use enigo::{Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use tracing::{debug, warn};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT};
use x11rb::rust_connection::RustConnection;

pub struct X11Injector {
    enigo: Enigo,
    /// 独立的 x11rb 连接，专门用于发滚轮 XTest 事件。
    conn: RustConnection,
}

impl X11Injector {
    /// 初始化 enigo 实例及用于滚轮注入的 x11rb 连接。
    /// 失败一般意味着无法连上 X server（无 DISPLAY 等）。
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .context("初始化 enigo（X11）失败：检查 DISPLAY 与 XTest 扩展")?;
        let (conn, _) = x11rb::connect(None)
            .context("滚轮注入：连接 X server 失败（检查 DISPLAY）")?;
        // 确认 XTEST 扩展存在，否则滚轮注入无意义。
        conn.extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
            .context("查询 XTEST 扩展失败")?
            .context("XTEST 扩展不可用，无法注入滚轮")?;
        Ok(Self { enigo, conn })
    }

    /// 注入 `count` 次指定方向的滚轮事件。
    ///
    /// 直接用 x11rb 发 XTest BUTTON_PRESS/RELEASE，`detail` 为滚轮按键号
    /// (4=上,5=下,6=左,7=右)，`deviceid` 传 0 复刻 xdotool 的默认设备行为。
    /// root 传 0（x11rb::NONE）表示事件落在指针当前所在屏幕。
    fn emit_scroll(&mut self, button: u8, count: u32) -> Result<()> {
        for _ in 0..count {
            x11rb::protocol::xtest::fake_input(
                &self.conn,
                BUTTON_PRESS_EVENT,
                button,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .map_err(|e| anyhow::anyhow!("滚轮 press 失败: {e:?}"))?;
            x11rb::protocol::xtest::fake_input(
                &self.conn,
                BUTTON_RELEASE_EVENT,
                button,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .map_err(|e| anyhow::anyhow!("滚轮 release 失败: {e:?}"))?;
        }
        self.conn
            .flush()
            .map_err(|e| anyhow::anyhow!("滚轮事件 flush 失败: {e:?}"))?;
        Ok(())
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
        // 这里把像素级 delta 简化为离散步数（每 100px 一步、至少 1 步）。
        let steps_y = scroll_steps(dy);
        let steps_x = scroll_steps(dx);

        if steps_y != 0 {
            // X11 滚轮按键号：4=上，5=下。dy>0 向下滚 = 5，dy<0 向上滚 = 4。
            let button = if steps_y > 0 { 5 } else { 4 };
            self.emit_scroll(button, steps_y.unsigned_abs())?;
        }
        if steps_x != 0 {
            // 6=左，7=右。dx>0 向右滚 = 7，dx<0 向左滚 = 6。
            let button = if steps_x > 0 { 7 } else { 6 };
            self.emit_scroll(button, steps_x.unsigned_abs())?;
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
