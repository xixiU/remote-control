//! 消息编解码（输入事件、视频帧、控制消息）
//! M1 实现：定义服务端↔浏览器的 JSON 控制消息，并提供编解码 helper。

use serde::{Deserialize, Serialize};

/// 服务端 → 浏览器 的控制消息。
///
/// 序列化为 `{ "type": "<snake_case>", ... }` 形式，与 docs/03 通信协议一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// 连接建立后服务端首先发送，声明能力。
    Hello {
        version: String,
        auth_required: bool,
        video_codec: String,
        delivery: String,
    },
    /// 认证成功，下发会话 ID。
    AuthOk { session_id: String },
    /// 屏幕信息（M3：认证后发送）
    ScreenInfo { width: u32, height: u32 },
    /// 光标形状（形状变化时发送）
    CursorShape {
        width: u16,
        height: u16,
        xhot: u16,
        yhot: u16,
        /// RGBA 像素数据（base64 编码）
        rgba_base64: String,
    },
    /// 认证失败，附带原因。
    AuthFail { reason: String },
    /// 通用错误。
    Error { code: String, message: String },
    /// 远端剪切板内容（响应客户端的 clipboard_get）。
    ClipboardData {
        text: String,
        /// 本次内容的指纹，客户端存下来用于下次请求时比对。
        #[serde(default)]
        fingerprint: String,
    },
    /// 被控端剪切板与客户端持有的内容一致，正文未重传。
    ///
    /// 只在客户端的 `clipboard_get` 带了匹配的 fingerprint 时发送，
    /// 用来省掉轮询重试中的重复下载。
    ClipboardUnchanged,
    /// 剪切板操作失败（区别于通用 Error，便于前端按按钮反馈）。
    /// `op` 取 "get" / "set"，方便客户端定位是哪边失败。
    ClipboardError { op: String, message: String },
    /// 心跳响应。
    Pong,
}

/// 浏览器 → 服务端 的控制消息。
///
/// 对未知/未来扩展的消息使用 `#[serde(other)]` 兜底变体 `Unknown`，
/// 避免反序列化失败导致连接被断开。注意：`Unknown` 变体不会捕获字段，
/// 因此 `Input`/`Control` 等需要在 M2/M3 处理字段的消息显式列出。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// 提交认证。
    Auth { token: String },
    /// 客户端控制请求（请求关键帧、调码率、切屏等）。M1 暂不处理。
    Control {
        /// 控制动作，宽松解析，M1 用不到。
        #[serde(default)]
        action: String,
    },
    /// 输入事件（鼠标/键盘）。M2 完整解析。
    Input {
        /// 事件子类型：mouse_move/mouse_down/mouse_up/wheel/key_down/key_up。
        #[serde(default)]
        event: String,
        /// 归一化 x 坐标 (0.0~1.0)，用于鼠标事件。
        #[serde(default)]
        x: f64,
        /// 归一化 y 坐标 (0.0~1.0)，用于鼠标事件。
        #[serde(default)]
        y: f64,
        /// 鼠标按键："left"/"right"/"middle"。
        #[serde(default)]
        button: String,
        /// 滚轮横向增量。
        #[serde(default)]
        dx: i32,
        /// 滚轮纵向增量。
        #[serde(default)]
        dy: i32,
        /// 键盘按键码（KeyboardEvent.code，如 "KeyA"）。
        #[serde(default)]
        code: String,
        /// 修饰键列表（"shift"/"ctrl"/"alt"/"meta"）。
        #[serde(default)]
        modifiers: Vec<String>,
    },
    /// 心跳。
    Ping,
    /// 把本地（控制端）剪切板的文本同步到被控端。
    ClipboardSet {
        #[serde(default)]
        text: String,
    },
    /// 请求被控端剪切板文本，服务端以 `ClipboardData` 回复。
    ClipboardGet {
        /// 客户端已持有内容的指纹（见 `ClipboardData::fingerprint`）。
        ///
        /// 带上它时，若被控端剪切板内容未变，服务端只回
        /// `ClipboardUnchanged` 而不重传正文 —— 自动同步要轮询等复制动作
        /// 落地，重试几次就把同一份文本重复下载几遍，公网小带宽下会挤掉视频流。
        /// 空字符串表示客户端没有可比对的内容，正常返回全文。
        #[serde(default)]
        fingerprint: String,
    },
    /// 发送特殊快捷键组合（浏览器无法捕获的系统键）。
    SpecialKey {
        /// 快捷键标识："alt_f4" / "ctrl_alt_del" 等。
        #[serde(default)]
        key: String,
    },
    /// 兜底：未识别的消息类型，记日志后忽略，不断连。
    #[serde(other)]
    Unknown,
}

impl ServerMsg {
    /// 将服务端消息序列化为 JSON 文本。
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

/// 算一段剪切板文本的指纹，用于判断"内容有没有变"。
///
/// 用标准库 hasher 而非引入 sha2：这里只需要在一次会话内比对两段文本是否相同，
/// 不是安全场景，不需要抗碰撞。`DefaultHasher` 不保证跨 Rust 版本稳定，
/// 但指纹由服务端生成、客户端原样回传、再由同一进程比对，不跨版本流转。
///
/// 带上长度一起输出，让碰撞也要同时撞长度，进一步降低误判概率。
pub fn clipboard_fingerprint(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:x}-{:x}", text.len(), hasher.finish())
}

/// 从文本解析浏览器消息。解析失败时返回错误，由调用方决定处理（M1 仅记日志）。
pub fn parse_client_msg(text: &str) -> serde_json::Result<ClientMsg> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_serializes_with_correct_fields() {
        let msg = ServerMsg::Hello {
            version: "1.0".to_string(),
            auth_required: true,
            video_codec: "h264".to_string(),
            delivery: "webcodecs".to_string(),
        };
        let json = msg.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["type"], "hello");
        assert_eq!(value["version"], "1.0");
        assert_eq!(value["auth_required"], true);
        assert_eq!(value["video_codec"], "h264");
        assert_eq!(value["delivery"], "webcodecs");
    }

    #[test]
    fn auth_ok_and_auth_fail_use_snake_case_type() {
        let ok = ServerMsg::AuthOk {
            session_id: "abc".to_string(),
        }
        .to_json()
        .unwrap();
        assert!(ok.contains("\"type\":\"auth_ok\""));
        assert!(ok.contains("\"session_id\":\"abc\""));

        let fail = ServerMsg::AuthFail {
            reason: "invalid_password".to_string(),
        }
        .to_json()
        .unwrap();
        assert!(fail.contains("\"type\":\"auth_fail\""));
        assert!(fail.contains("invalid_password"));
    }

    #[test]
    fn pong_serializes_to_type_only() {
        let json = ServerMsg::Pong.to_json().unwrap();
        assert_eq!(json, "{\"type\":\"pong\"}");
    }

    #[test]
    fn auth_message_parses() {
        let msg = parse_client_msg(r#"{"type":"auth","token":"secret"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::Auth {
                token: "secret".to_string()
            }
        );
    }

    #[test]
    fn ping_message_parses() {
        let msg = parse_client_msg(r#"{"type":"ping"}"#).unwrap();
        assert_eq!(msg, ClientMsg::Ping);
    }

    #[test]
    fn input_and_control_parse_loosely() {
        let input = parse_client_msg(
            r#"{"type":"input","event":"mouse_move","x":0.5,"y":0.5}"#,
        )
        .unwrap();
        assert_eq!(
            input,
            ClientMsg::Input {
                event: "mouse_move".to_string(),
                x: 0.5,
                y: 0.5,
                button: String::new(),
                dx: 0,
                dy: 0,
                code: String::new(),
                modifiers: Vec::new(),
            }
        );

        let control =
            parse_client_msg(r#"{"type":"control","action":"request_keyframe"}"#).unwrap();
        assert_eq!(
            control,
            ClientMsg::Control {
                action: "request_keyframe".to_string()
            }
        );
    }

    #[test]
    fn input_mouse_down_parses_button() {
        let msg = parse_client_msg(
            r#"{"type":"input","event":"mouse_down","button":"right","x":0.1,"y":0.2}"#,
        )
        .unwrap();
        match msg {
            ClientMsg::Input {
                event,
                button,
                x,
                y,
                ..
            } => {
                assert_eq!(event, "mouse_down");
                assert_eq!(button, "right");
                assert!((x - 0.1).abs() < 1e-9);
                assert!((y - 0.2).abs() < 1e-9);
            }
            _ => panic!("应解析为 Input"),
        }
    }

    #[test]
    fn input_wheel_parses_deltas() {
        let msg = parse_client_msg(r#"{"type":"input","event":"wheel","dx":0,"dy":-120}"#)
            .unwrap();
        match msg {
            ClientMsg::Input { event, dx, dy, .. } => {
                assert_eq!(event, "wheel");
                assert_eq!(dx, 0);
                assert_eq!(dy, -120);
            }
            _ => panic!("应解析为 Input"),
        }
    }

    #[test]
    fn input_key_event_parses_code_and_modifiers() {
        let msg = parse_client_msg(
            r#"{"type":"input","event":"key_down","code":"KeyA","modifiers":["shift","ctrl"]}"#,
        )
        .unwrap();
        match msg {
            ClientMsg::Input {
                event,
                code,
                modifiers,
                ..
            } => {
                assert_eq!(event, "key_down");
                assert_eq!(code, "KeyA");
                assert_eq!(modifiers, vec!["shift".to_string(), "ctrl".to_string()]);
            }
            _ => panic!("应解析为 Input"),
        }
    }

    #[test]
    fn unknown_message_does_not_panic() {
        // 未知 type 走兜底变体，不报错、不断连。
        let msg = parse_client_msg(r#"{"type":"some_future_type","foo":42}"#).unwrap();
        assert_eq!(msg, ClientMsg::Unknown);

        // 缺少可选字段也不应失败。
        let msg2 = parse_client_msg(r#"{"type":"control"}"#).unwrap();
        assert_eq!(
            msg2,
            ClientMsg::Control {
                action: String::new()
            }
        );
    }

    #[test]
    fn clipboard_messages_roundtrip() {
        let set = parse_client_msg(r#"{"type":"clipboard_set","text":"hello"}"#).unwrap();
        assert_eq!(
            set,
            ClientMsg::ClipboardSet {
                text: "hello".to_string()
            }
        );

        // 不带 fingerprint 的老客户端消息仍能解析（serde(default) 兜底）。
        let get = parse_client_msg(r#"{"type":"clipboard_get"}"#).unwrap();
        assert_eq!(
            get,
            ClientMsg::ClipboardGet {
                fingerprint: String::new()
            }
        );

        let get_fp = parse_client_msg(r#"{"type":"clipboard_get","fingerprint":"5-abc"}"#).unwrap();
        assert_eq!(
            get_fp,
            ClientMsg::ClipboardGet {
                fingerprint: "5-abc".to_string()
            }
        );

        let data = ServerMsg::ClipboardData {
            text: "world".to_string(),
            fingerprint: clipboard_fingerprint("world"),
        }
        .to_json()
        .unwrap();
        assert!(data.contains("\"type\":\"clipboard_data\""));
        assert!(data.contains("\"text\":\"world\""));
        assert!(data.contains("\"fingerprint\":"));

        let unchanged = ServerMsg::ClipboardUnchanged.to_json().unwrap();
        assert!(unchanged.contains("\"type\":\"clipboard_unchanged\""));

        let err = ServerMsg::ClipboardError {
            op: "set".to_string(),
            message: "no x11".to_string(),
        }
        .to_json()
        .unwrap();
        assert!(err.contains("\"type\":\"clipboard_error\""));
        assert!(err.contains("\"op\":\"set\""));
    }

    #[test]
    fn clipboard_fingerprint_distinguishes_content() {
        // 同内容同指纹，是"未变则不重传"的前提。
        assert_eq!(clipboard_fingerprint("hello"), clipboard_fingerprint("hello"));
        assert_ne!(clipboard_fingerprint("hello"), clipboard_fingerprint("world"));
        // 等长不同内容也要能区分（指纹不是只看长度）。
        assert_ne!(clipboard_fingerprint("aaaa"), clipboard_fingerprint("bbbb"));
        // 空内容也有稳定指纹，但客户端用空串表示"没有可比对内容"，二者不冲突。
        assert!(!clipboard_fingerprint("").is_empty());
    }

    #[test]
    fn special_key_message_parses() {
        let msg = parse_client_msg(r#"{"type":"special_key","key":"alt_f4"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::SpecialKey {
                key: "alt_f4".to_string()
            }
        );

        let msg2 = parse_client_msg(r#"{"type":"special_key","key":"ctrl_alt_del"}"#).unwrap();
        match msg2 {
            ClientMsg::SpecialKey { key } => {
                assert_eq!(key, "ctrl_alt_del");
            }
            _ => panic!("应解析为 SpecialKey"),
        }
    }
}
