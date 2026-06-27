//! Windows 输入注入实现（enigo + SendInput 混合方案）
//!
//! 鼠标移动/点击/键盘：使用 enigo crate
//! 滚轮：直接调用 Windows SendInput API，支持精确滚动量控制

use super::{keymap, InputInjector, MouseButton};
use anyhow::Result;
use enigo::{Button, Coordinate, Direction, Enigo, Keyboard, Mouse};
use tracing::{debug, warn};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
};

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

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()> {
        // 纵向滚轮
        if dy != 0 {
            let amount = calculate_wheel_delta(dy);
            send_wheel_event(amount, false)?;
            debug!("纵向滚轮: dy={} -> amount={}", dy, amount);
        }

        // 横向滚轮
        if dx != 0 {
            let amount = calculate_wheel_delta(dx);
            send_wheel_event(amount, true)?;
            debug!("横向滚轮: dx={} -> amount={}", dx, amount);
        }

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
        } else {
            warn!("未支持的键码 '{}', 已忽略", code);
        }
        Ok(())
    }
}

/// 将浏览器 wheel delta（像素）转换为 Windows WHEEL_DELTA 单位
///
/// Windows WHEEL_DELTA = 120 表示滚动一格
/// 浏览器 deltaY 通常 100~150 像素为一次滚动
///
/// **方向说明：**
/// - 浏览器: deltaY > 0 向下滚动，deltaY < 0 向上滚动
/// - Windows: WHEEL_DELTA > 0 向上滚动，WHEEL_DELTA < 0 向下滚动
/// - 因此需要反转方向
///
/// 简化转换：每 120 像素 = 一个 WHEEL_DELTA
/// 小量滚动至少触发一个 delta，大量滚动限制在最多 10 格
#[cfg(target_os = "windows")]
fn calculate_wheel_delta(delta: i32) -> i32 {
    if delta == 0 {
        return 0;
    }

    // Windows WHEEL_DELTA = 120 是一"格"滚动
    // 浏览器 delta 通常以像素为单位，约 100~150px 一格
    const WHEEL_DELTA: i32 = 120;
    const PIXELS_PER_STEP: i32 = 120;

    let steps = delta / PIXELS_PER_STEP;

    // 反转方向：浏览器向下(delta>0) → Windows 向下(amount<0)
    let result = if steps == 0 {
        // 小量滚动至少触发一格
        if delta > 0 {
            -WHEEL_DELTA  // 向下
        } else {
            WHEEL_DELTA   // 向上
        }
    } else {
        // 限制范围：最多 10 格，反转方向
        (-steps * WHEEL_DELTA).clamp(-WHEEL_DELTA * 10, WHEEL_DELTA * 10)
    };

    result
}

/// 发送滚轮事件到 Windows
///
/// # 参数
/// - `amount`: 滚动量（正数向上/右，负数向下/左），单位是 WHEEL_DELTA
/// - `is_horizontal`: true = 横向滚轮，false = 纵向滚轮
#[cfg(target_os = "windows")]
fn send_wheel_event(amount: i32, is_horizontal: bool) -> Result<()> {
    // mouseData 是 u32，需要将有符号 i32 直接转译以保留符号位
    // Windows API: mouseData 使用有符号的 WHEEL_DELTA，正数向上/右，负数向下/左
    let input = if is_horizontal {
        // 横向滚轮
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: amount as i32 as u32, // 直接转译，保留符号位
                    dwFlags: MOUSEEVENTF_HWHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    } else {
        // 纵向滚轮
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: amount as i32 as u32, // 直接转译，保留符号位
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    };

    // 调用 SendInput
    unsafe {
        let sent = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        if sent == 0 {
            let err = std::io::Error::last_os_error();
            return Err(anyhow::anyhow!("SendInput 失败: {}", err));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn test_calculate_wheel_delta_zero() {
        assert_eq!(calculate_wheel_delta(0), 0);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_calculate_wheel_delta_small_positive() {
        // 浏览器向下滚动(delta>0) → Windows 向下滚动(amount<0)
        assert_eq!(calculate_wheel_delta(40), -120);
        assert_eq!(calculate_wheel_delta(80), -120);
        assert_eq!(calculate_wheel_delta(119), -120);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_calculate_wheel_delta_small_negative() {
        // 浏览器向上滚动(delta<0) → Windows 向上滚动(amount>0)
        assert_eq!(calculate_wheel_delta(-40), 120);
        assert_eq!(calculate_wheel_delta(-80), 120);
        assert_eq!(calculate_wheel_delta(-119), 120);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_calculate_wheel_delta_normal() {
        // 正常滚动量，方向反转
        assert_eq!(calculate_wheel_delta(120), -120);
        assert_eq!(calculate_wheel_delta(240), -240);
        assert_eq!(calculate_wheel_delta(-240), 240);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_calculate_wheel_delta_large() {
        // 大量滚动 → 限制在 10 格，方向反转
        assert_eq!(calculate_wheel_delta(100_000), -1200);
        assert_eq!(calculate_wheel_delta(-100_000), 1200);
        // 恰好 10 格
        assert_eq!(calculate_wheel_delta(1200), -1200);
        assert_eq!(calculate_wheel_delta(-1200), 1200);
    }
}