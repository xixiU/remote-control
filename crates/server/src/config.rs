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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    /// 目标帧率
    pub fps: u32,
    /// 目标码率 (bps)
    pub bitrate: u32,
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
        }
    }
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            fps: 30,
            bitrate: 4_000_000,
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
