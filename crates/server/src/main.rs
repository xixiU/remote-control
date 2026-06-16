mod config;
mod web;
mod session;
mod protocol;
mod capture;
mod encoder;
mod input;
mod pipeline;

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;

#[derive(Parser, Debug)]
#[command(name = "remotecontrol-server", about = "RemoteControl 被控端服务")]
struct Cli {
    /// 监听地址
    #[arg(long, default_value = "0.0.0.0:8443")]
    bind: String,

    /// 访问密码
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

    // 加载配置：配置文件 + 命令行覆盖
    let mut config = AppConfig::load(&cli.config).unwrap_or_default();
    if let Some(ref bind) = Some(cli.bind.clone()) {
        config.bind = bind.clone();
    }
    if let Some(ref password) = cli.password {
        config.password = password.clone();
    }

    info!("RemoteControl 被控端服务启动中...");
    info!("监听地址: {}", config.bind);

    // 启动 HTTP 服务
    web::serve(config).await?;

    Ok(())
}
