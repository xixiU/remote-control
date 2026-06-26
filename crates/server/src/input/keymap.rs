//! KeyboardEvent.code → enigo::Key 映射表
//!
//! 浏览器 `KeyboardEvent.code` 表示物理按键位置，与键盘布局无关，
//! 因此服务端按 code 映射到平台键码更可靠。
//!
//! 见 docs/03-通信协议.md 第 3.3/3.4 节。

use enigo::Key;

/// 把浏览器 `KeyboardEvent.code` 映射到 enigo `Key`。
///
/// 返回 `None` 表示当前未支持该键，调用方应记录日志并忽略该事件。
#[must_use]
pub fn code_to_key(code: &str) -> Option<Key> {
    // 字母 KeyA-KeyZ → Unicode('a'..'z')。enigo 在 X11 上对 Unicode 字母做合理处理；
    // 大小写由 ShiftLeft/ShiftRight 状态控制，而不是直接发 'A'。
    if let Some(rest) = code.strip_prefix("Key") {
        if rest.len() == 1 {
            let ch = rest.chars().next().unwrap();
            if ch.is_ascii_uppercase() {
                return Some(Key::Unicode(ch.to_ascii_lowercase()));
            }
        }
    }

    // 数字 Digit0-Digit9 → Unicode('0'..'9')。
    if let Some(rest) = code.strip_prefix("Digit") {
        if rest.len() == 1 {
            let ch = rest.chars().next().unwrap();
            if ch.is_ascii_digit() {
                return Some(Key::Unicode(ch));
            }
        }
    }

    // 小键盘 Numpad0-Numpad9 → 专用 Numpad 键。
    if let Some(rest) = code.strip_prefix("Numpad") {
        match rest {
            "0" => return Some(Key::Numpad0),
            "1" => return Some(Key::Numpad1),
            "2" => return Some(Key::Numpad2),
            "3" => return Some(Key::Numpad3),
            "4" => return Some(Key::Numpad4),
            "5" => return Some(Key::Numpad5),
            "6" => return Some(Key::Numpad6),
            "7" => return Some(Key::Numpad7),
            "8" => return Some(Key::Numpad8),
            "9" => return Some(Key::Numpad9),
            "Add" => return Some(Key::Add),
            "Subtract" => return Some(Key::Subtract),
            "Multiply" => return Some(Key::Multiply),
            "Divide" => return Some(Key::Divide),
            "Decimal" => return Some(Key::Decimal),
            "Enter" => return Some(Key::Return),
            _ => {}
        }
    }

    // 功能键 F1-F12（保留 F13-F24 兼容性）。
    if let Some(rest) = code.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u32>() {
            return match n {
                1 => Some(Key::F1),
                2 => Some(Key::F2),
                3 => Some(Key::F3),
                4 => Some(Key::F4),
                5 => Some(Key::F5),
                6 => Some(Key::F6),
                7 => Some(Key::F7),
                8 => Some(Key::F8),
                9 => Some(Key::F9),
                10 => Some(Key::F10),
                11 => Some(Key::F11),
                12 => Some(Key::F12),
                13 => Some(Key::F13),
                14 => Some(Key::F14),
                15 => Some(Key::F15),
                16 => Some(Key::F16),
                17 => Some(Key::F17),
                18 => Some(Key::F18),
                19 => Some(Key::F19),
                20 => Some(Key::F20),
                _ => None,
            };
        }
    }

    match code {
        // 方向键
        "ArrowUp" => Some(Key::UpArrow),
        "ArrowDown" => Some(Key::DownArrow),
        "ArrowLeft" => Some(Key::LeftArrow),
        "ArrowRight" => Some(Key::RightArrow),

        // 修饰键（区分左右）
        "ShiftLeft" => Some(Key::LShift),
        "ShiftRight" => Some(Key::RShift),
        "ControlLeft" => Some(Key::LControl),
        "ControlRight" => Some(Key::RControl),
        "AltLeft" | "AltRight" => Some(Key::Alt),
        "MetaLeft" | "MetaRight" | "OSLeft" | "OSRight" => Some(Key::Meta),

        // 编辑/导航
        "Enter" => Some(Key::Return),
        "Backspace" => Some(Key::Backspace),
        "Tab" => Some(Key::Tab),
        "Space" => Some(Key::Space),
        "Escape" => Some(Key::Escape),
        "Delete" => Some(Key::Delete),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "PageUp" => Some(Key::PageUp),
        "PageDown" => Some(Key::PageDown),
        "Insert" => Some(Key::Insert),

        // 锁定/系统
        "CapsLock" => Some(Key::CapsLock),
        "NumLock" => Some(Key::Numlock),
        #[cfg(target_os = "windows")]
        "ScrollLock" => Some(Key::Scroll),
        #[cfg(all(unix, not(target_os = "macos")))]
        "ScrollLock" => Some(Key::ScrollLock),
        "Pause" => Some(Key::Pause),
        "PrintScreen" => Some(Key::PrintScr),

        // 常用符号（按 US 键盘的物理位置映射到对应字符）。
        // 实际显示字符取决于按键时是否按下 Shift；这里只保证物理键被按下。
        "Minus" => Some(Key::Unicode('-')),
        "Equal" => Some(Key::Unicode('=')),
        "BracketLeft" => Some(Key::Unicode('[')),
        "BracketRight" => Some(Key::Unicode(']')),
        "Backslash" => Some(Key::Unicode('\\')),
        "Semicolon" => Some(Key::Unicode(';')),
        "Quote" => Some(Key::Unicode('\'')),
        "Backquote" => Some(Key::Unicode('`')),
        "Comma" => Some(Key::Unicode(',')),
        "Period" => Some(Key::Unicode('.')),
        "Slash" => Some(Key::Unicode('/')),
        "IntlBackslash" => Some(Key::Unicode('\\')),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unicode(ch: char) -> Key {
        Key::Unicode(ch)
    }

    #[test]
    fn letters_map_to_lowercase_unicode() {
        assert_eq!(code_to_key("KeyA"), Some(unicode('a')));
        assert_eq!(code_to_key("KeyM"), Some(unicode('m')));
        assert_eq!(code_to_key("KeyZ"), Some(unicode('z')));
    }

    #[test]
    fn digits_map_to_unicode() {
        assert_eq!(code_to_key("Digit0"), Some(unicode('0')));
        assert_eq!(code_to_key("Digit5"), Some(unicode('5')));
        assert_eq!(code_to_key("Digit9"), Some(unicode('9')));
    }

    #[test]
    fn numpad_digits_map_to_numpad_keys() {
        assert_eq!(code_to_key("Numpad0"), Some(Key::Numpad0));
        assert_eq!(code_to_key("Numpad9"), Some(Key::Numpad9));
        assert_eq!(code_to_key("NumpadEnter"), Some(Key::Return));
        assert_eq!(code_to_key("NumpadAdd"), Some(Key::Add));
    }

    #[test]
    fn function_keys_map() {
        assert_eq!(code_to_key("F1"), Some(Key::F1));
        assert_eq!(code_to_key("F12"), Some(Key::F12));
    }

    #[test]
    fn arrow_keys_map() {
        assert_eq!(code_to_key("ArrowUp"), Some(Key::UpArrow));
        assert_eq!(code_to_key("ArrowDown"), Some(Key::DownArrow));
        assert_eq!(code_to_key("ArrowLeft"), Some(Key::LeftArrow));
        assert_eq!(code_to_key("ArrowRight"), Some(Key::RightArrow));
    }

    #[test]
    fn modifiers_map_distinguishing_left_right() {
        assert_eq!(code_to_key("ShiftLeft"), Some(Key::LShift));
        assert_eq!(code_to_key("ShiftRight"), Some(Key::RShift));
        assert_eq!(code_to_key("ControlLeft"), Some(Key::LControl));
        assert_eq!(code_to_key("ControlRight"), Some(Key::RControl));
        assert_eq!(code_to_key("AltLeft"), Some(Key::Alt));
        assert_eq!(code_to_key("AltRight"), Some(Key::Alt));
        assert_eq!(code_to_key("MetaLeft"), Some(Key::Meta));
    }

    #[test]
    fn special_keys_map() {
        assert_eq!(code_to_key("Enter"), Some(Key::Return));
        assert_eq!(code_to_key("Backspace"), Some(Key::Backspace));
        assert_eq!(code_to_key("Tab"), Some(Key::Tab));
        assert_eq!(code_to_key("Space"), Some(Key::Space));
        assert_eq!(code_to_key("Escape"), Some(Key::Escape));
        assert_eq!(code_to_key("Delete"), Some(Key::Delete));
    }

    #[test]
    fn navigation_keys_map() {
        assert_eq!(code_to_key("Home"), Some(Key::Home));
        assert_eq!(code_to_key("End"), Some(Key::End));
        assert_eq!(code_to_key("PageUp"), Some(Key::PageUp));
        assert_eq!(code_to_key("PageDown"), Some(Key::PageDown));
        assert_eq!(code_to_key("Insert"), Some(Key::Insert));
    }

    #[test]
    fn symbols_map_to_unicode() {
        assert_eq!(code_to_key("Minus"), Some(unicode('-')));
        assert_eq!(code_to_key("Equal"), Some(unicode('=')));
        assert_eq!(code_to_key("Comma"), Some(unicode(',')));
        assert_eq!(code_to_key("Period"), Some(unicode('.')));
        assert_eq!(code_to_key("Slash"), Some(unicode('/')));
    }

    #[test]
    fn unmapped_returns_none() {
        assert_eq!(code_to_key(""), None);
        assert_eq!(code_to_key("KeyAA"), None);
        assert_eq!(code_to_key("Digit10"), None);
        assert_eq!(code_to_key("F999"), None);
        assert_eq!(code_to_key("UnknownKey"), None);
    }
}
