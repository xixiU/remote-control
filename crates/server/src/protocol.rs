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
    /// 认证失败，附带原因。
    AuthFail { reason: String },
    /// 通用错误。
    Error { code: String, message: String },
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
    /// 输入事件（鼠标/键盘）。M1 暂不处理，留给 M2。
    Input {
        /// 事件子类型，宽松解析。
        #[serde(default)]
        event: String,
    },
    /// 心跳。
    Ping,
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
                event: "mouse_move".to_string()
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
}
