use serde::Deserialize;
use std::fs;
use std::path::Path;
use tracing::warn;

/// 应用配置
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 监听地址
    pub bind: String,
    /// 访问密码（明文）。若同时提供 password_hash，则以 password_hash 优先。
    pub password: String,
    /// 访问密码的 argon2 哈希（PHC 格式）。配置直接给哈希时使用，优先于明文 password。
    pub password_hash: Option<String>,
    /// 视频配置
    pub video: VideoConfig,
    /// 安全配置
    pub security: SecurityConfig,
    /// TLS 配置
    pub tls: TlsConfig,
    /// 剪切板配置
    pub clipboard: ClipboardConfig,
}

/// 剪切板同步配置
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// 是否允许剪切板同步（默认 true）。关闭后客户端的 clipboard_get/set 会被拒绝。
    pub enabled: bool,
    /// 单次文本同步的最大字节数，超出截断或拒绝（默认 1 MB）。
    pub max_text_bytes: usize,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_text_bytes: 1024 * 1024,
        }
    }
}

/// TLS 配置
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    /// 是否启用 TLS（默认 true）
    pub enabled: bool,
    /// 证书文件路径（留空则自动生成自签名证书）
    pub cert: Option<String>,
    /// 私钥文件路径
    pub key: Option<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cert: None,
            key: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    /// 目标帧率
    pub fps: u32,
    /// 目标码率 (Mbps)。设为 -1（默认）时按分辨率与帧率自动估算。
    pub bitrate: i32,
    /// 编码器
    pub codec: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// 最大认证失败次数
    pub max_auth_failures: u32,
    /// 认证封禁时间（秒）
    pub auth_ban_seconds: u64,
    /// 会话空闲超时（秒）
    pub session_idle_timeout: u64,
    /// 独占控制
    pub exclusive_control: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8443".to_string(),
            password: "123456".to_string(),
            password_hash: None,
            video: VideoConfig::default(),
            security: SecurityConfig::default(),
            tls: TlsConfig::default(),
            clipboard: ClipboardConfig::default(),
        }
    }
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            fps: 30,
            bitrate: -1,
            codec: "h264".to_string(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_auth_failures: 5,
            auth_ban_seconds: 300,
            session_idle_timeout: 300,
            exclusive_control: false,
        }
    }
}

impl AppConfig {
    /// 从 TOML 文件加载配置
    pub fn load(path: &str) -> Option<Self> {
        let path = Path::new(path);
        if !path.exists() {
            warn!("配置文件 {} 不存在，使用默认配置", path.display());
            return None;
        }
        let content = fs::read_to_string(path).ok()?;
        match toml::from_str(&content) {
            Ok(config) => Some(config),
            Err(e) => {
                warn!("解析配置文件失败: {}，使用默认配置", e);
                None
            }
        }
    }
}
