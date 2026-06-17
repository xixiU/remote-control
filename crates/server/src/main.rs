mod config;
mod tls;
mod web;
mod session;
mod protocol;
mod capture;
mod encoder;
mod input;
mod pipeline;
mod clipboard;

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;
use crate::session::Authenticator;

#[derive(Parser, Debug)]
#[command(name = "remotecontrol-server", about = "RemoteControl 被控端服务")]
struct Cli {
    /// 监听地址（仅在显式提供时覆盖配置文件）
    #[arg(long)]
    bind: Option<String>,

    /// 访问密码（仅在显式提供时覆盖配置文件）
    #[arg(long)]
    password: Option<String>,

    /// 配置文件路径
    #[arg(long, default_value = "config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // 加载配置：配置文件 + 命令行覆盖（仅命令行显式提供时才覆盖）
    let mut config = AppConfig::load(&cli.config).unwrap_or_default();
    if let Some(bind) = cli.bind {
        config.bind = bind;
    }
    if let Some(password) = cli.password {
        config.password = password;
        // 命令行明文密码优先级高于配置文件里的哈希。
        config.password_hash = None;
    }

    // 初始化密码校验器：password_hash 优先于明文 password。
    let authenticator = match config.password_hash.as_deref() {
        Some(hash) => Authenticator::from_hash(hash)?,
        None => Authenticator::from_password(&config.password)?,
    };

    info!("RemoteControl 被控端服务启动中...");
    info!("监听地址: {}", config.bind);

    // 启动 HTTP 服务
    web::serve(config, authenticator).await?;

    Ok(())
}
