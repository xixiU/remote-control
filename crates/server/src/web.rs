//! axum 路由、静态资源托管、WebSocket 升级与认证处理
//! M1 实现：/ws 升级、hello、认证状态机循环。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tracing::{debug, info, warn};

use crate::config::AppConfig;
use crate::protocol::{parse_client_msg, ClientMsg, ServerMsg};
use crate::session::{Authenticator, AuthState, BruteForceGuard, Session};

/// 认证失败时固定延迟回复，削弱时序侧信道并拖慢爆破（见 docs/04 第 4 节）。
const AUTH_FAIL_DELAY: Duration = Duration::from_millis(800);

/// 全局共享状态，通过 axum 的 with_state 注入到各连接。
pub struct AppState {
    pub config: AppConfig,
    pub authenticator: Authenticator,
    pub guard: BruteForceGuard,
}

/// 启动 HTTP 服务器：托管静态资源 + /ws WebSocket。
pub async fn serve(config: AppConfig, authenticator: Authenticator) -> Result<()> {
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

    let guard = BruteForceGuard::new(
        config.security.max_auth_failures,
        config.security.auth_ban_seconds,
    );

    let bind = config.bind.clone();
    let state = Arc::new(AppState {
        config,
        authenticator,
        guard,
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new(&web_dir))
        .with_state(state);

    let listener = TcpListener::bind(&bind).await?;
    info!("HTTP 服务已启动: http://{}", bind);

    // 监听 0.0.0.0 时打印安全提示（见 docs/04 第 6 节）。
    if bind.starts_with("0.0.0.0") {
        warn!(
            "正在监听所有网卡 (0.0.0.0)，服务暴露于整个局域网。请勿直接暴露到公网，并设置足够复杂的密码。"
        );
    }

    // 需要 ConnectInfo 拿到客户端 IP，故用 into_make_service_with_connect_info。
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// /ws 路由：升级为 WebSocket，并把客户端地址带入处理逻辑。
async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, peer_addr, state))
}

/// 单个 WebSocket 连接的处理 task。连接断开即结束（会话随之销毁）。
async fn handle_socket(mut socket: WebSocket, peer_addr: SocketAddr, state: Arc<AppState>) {
    info!("WebSocket 连接建立: {}", peer_addr);

    // 1. 连接建立后立即发送 hello。
    let hello = ServerMsg::Hello {
        version: "1.0".to_string(),
        auth_required: true,
        video_codec: "h264".to_string(),
        delivery: "webcodecs".to_string(),
    };
    if send_msg(&mut socket, &hello).await.is_err() {
        warn!("{}: 发送 hello 失败，关闭连接", peer_addr);
        return;
    }

    // 2. 未认证状态循环：只接受 auth / ping。
    let mut state_machine = AuthState::Unauthenticated;
    let mut session: Option<Session> = None;

    while let Some(frame) = socket.recv().await {
        let msg = match frame {
            Ok(Message::Text(text)) => match parse_client_msg(&text) {
                Ok(m) => m,
                Err(e) => {
                    // 解析失败只记 warn，不断连。
                    warn!("{}: 消息解析失败，已忽略: {}", peer_addr, e);
                    continue;
                }
            },
            Ok(Message::Close(_)) => {
                debug!("{}: 收到 Close 帧", peer_addr);
                break;
            }
            // axum 会自动回应底层 Ping/Pong，这里无需处理。
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(Message::Binary(_)) => {
                debug!("{}: 收到二进制帧（M1 暂不处理）", peer_addr);
                continue;
            }
            Err(e) => {
                warn!("{}: WebSocket 接收错误: {}", peer_addr, e);
                break;
            }
        };

        match &state_machine {
            AuthState::Unauthenticated => {
                match msg {
                    ClientMsg::Auth { token } => {
                        // 先查封禁。
                        if let Some(remaining) = state.guard.is_banned(peer_addr.ip()) {
                            warn!(
                                "{}: 来源 IP 已被封禁，拒绝认证（剩余 {}s）",
                                peer_addr,
                                remaining.as_secs()
                            );
                            let reason = format!(
                                "too_many_failures: 请 {} 秒后再试",
                                remaining.as_secs().max(1)
                            );
                            let _ = send_msg(&mut socket, &ServerMsg::AuthFail { reason }).await;
                            break;
                        }

                        if state.authenticator.verify(&token) {
                            // 认证成功。
                            state.guard.record_success(peer_addr.ip());
                            let new_session = Session::new(peer_addr);
                            let session_id = new_session.id;
                            info!("{}: 认证成功，会话 {}", peer_addr, session_id);
                            if send_msg(
                                &mut socket,
                                &ServerMsg::AuthOk {
                                    session_id: session_id.to_string(),
                                },
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                            state_machine = AuthState::Authenticated { session_id };
                            session = Some(new_session);
                        } else {
                            // 认证失败：计数 + 固定延迟后回复。
                            state.guard.record_failure(peer_addr.ip());
                            warn!("{}: 认证失败（密码错误）", peer_addr);
                            tokio::time::sleep(AUTH_FAIL_DELAY).await;
                            let _ = send_msg(
                                &mut socket,
                                &ServerMsg::AuthFail {
                                    reason: "invalid_password".to_string(),
                                },
                            )
                            .await;
                        }
                    }
                    ClientMsg::Ping => {
                        let _ = send_msg(&mut socket, &ServerMsg::Pong).await;
                    }
                    other => {
                        // 未认证状态下丢弃所有输入/控制消息（见 docs/04 第 5 节）。
                        debug!("{}: 未认证状态忽略消息: {:?}", peer_addr, other);
                    }
                }
            }
            AuthState::Authenticated { .. } => {
                if let Some(s) = session.as_mut() {
                    s.touch();
                }
                match msg {
                    ClientMsg::Ping => {
                        let _ = send_msg(&mut socket, &ServerMsg::Pong).await;
                    }
                    ClientMsg::Input { event } => {
                        // 真正处理留给 M2。
                        debug!("{}: 收到 input 事件 '{}'（M2 处理）", peer_addr, event);
                    }
                    ClientMsg::Control { action } => {
                        // 真正处理留给 M3。
                        debug!("{}: 收到 control '{}'（M3 处理）", peer_addr, action);
                    }
                    ClientMsg::Auth { .. } => {
                        debug!("{}: 已认证连接重复发送 auth，忽略", peer_addr);
                    }
                    ClientMsg::Unknown => {
                        debug!("{}: 已认证连接收到未知消息，忽略", peer_addr);
                    }
                }
            }
        }
    }

    info!("WebSocket 连接关闭: {}", peer_addr);
}

/// 序列化并发送一条服务端消息。
async fn send_msg(socket: &mut WebSocket, msg: &ServerMsg) -> Result<()> {
    let text = msg.to_json()?;
    socket.send(Message::Text(text)).await?;
    Ok(())
}
