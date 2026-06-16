use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tracing::info;

use crate::config::AppConfig;

/// 启动 HTTP 服务器，托管静态资源
pub async fn serve(config: AppConfig) -> Result<()> {
    // 静态资源目录：优先使用运行目录下的 web/，方便开发
    let web_dir = std::env::current_dir()?.join("web");
    let web_dir = if web_dir.exists() {
        web_dir
    } else {
        // fallback: 可执行文件同级的 web/ 目录
        let exe_dir = std::env::current_exe()?
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        exe_dir.join("web")
    };

    info!("静态资源目录: {}", web_dir.display());

    let app = Router::new()
        .fallback_service(ServeDir::new(&web_dir));

    let listener = TcpListener::bind(&config.bind).await?;
    info!("HTTP 服务已启动: http://{}", config.bind);

    axum::serve(listener, app).await?;

    Ok(())
}
