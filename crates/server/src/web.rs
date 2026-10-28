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

use crate::capture::{CursorShape, ScreenInfo};
use crate::clipboard;
use crate::config::AppConfig;
use crate::input::{self, InputInjector, MouseButton};
use crate::pipeline::CaptureEncoder;
use crate::protocol::{parse_client_msg, ClientMsg, ServerMsg};
use crate::session::{AuthState, Authenticator, BruteForceGuard, Session};

/// 认证失败时固定延迟回复，削弱时序侧信道并拖慢爆破（见 docs/04 第 4 节）。
const AUTH_FAIL_DELAY: Duration = Duration::from_millis(800);

/// 全局共享状态,通过 axum 的 with_state 注入到各连接。
pub struct AppState {
    pub config: AppConfig,
    pub authenticator: Authenticator,
    pub guard: BruteForceGuard,
    /// 视频帧 broadcast sender（M3）
    pub video_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    /// 关键帧请求 sender（M3）
    pub keyframe_tx: tokio::sync::mpsc::UnboundedSender<()>,
    /// 屏幕信息变化 broadcast sender
    pub screen_info_tx: tokio::sync::broadcast::Sender<crate::capture::ScreenInfo>,
    /// 屏幕信息（M3）- 使用 Arc<RwLock> 支持动态更新
    pub screen_info: Arc<tokio::sync::RwLock<crate::capture::ScreenInfo>>,
    /// 光标形状 broadcast sender
    pub cursor_tx: tokio::sync::broadcast::Sender<CursorShape>,
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

    // M3: 启动捕获编码管线
    // 配置里的码率单位是 Mbps，转成编码器使用的 bps；-1（自动）原样透传。
    let bitrate_bps = if config.video.bitrate <= 0 {
        -1
    } else {
        config.video.bitrate * 1_000_000
    };
    let (pipeline, screen_info) = CaptureEncoder::new(config.video.fps, bitrate_bps)?;
    let (video_tx, keyframe_tx, mut screen_info_rx, cursor_tx) =
        pipeline.start(config.video.fps, bitrate_bps, 8);

    info!("视频流已启动: {}x{}", screen_info.width, screen_info.height);

    let bind = config.bind.clone();
    let tls_enabled = config.tls.enabled;
    let tls_config = config.tls.clone();

    // 创建屏幕信息变化广播 channel
    let (screen_info_tx, _) = tokio::sync::broadcast::channel(16);
    let screen_info_tx_clone = screen_info_tx.clone();
    let screen_info_arc = Arc::new(tokio::sync::RwLock::new(screen_info.clone()));
    let screen_info_arc_clone = screen_info_arc.clone();

    // 启动屏幕信息变化监听任务
    tokio::spawn(async move {
        while let Some(new_info) = screen_info_rx.recv().await {
            info!(
                "屏幕分辨率已更新: {}x{}，通知所有客户端",
                new_info.width, new_info.height
            );
            // 更新共享的屏幕信息
            *screen_info_arc_clone.write().await = new_info.clone();
            // 广播给所有已连接的客户端
            let _ = screen_info_tx_clone.send(new_info);
        }
    });

    let state = Arc::new(AppState {
        config,
        authenticator,
        guard,
        video_tx,
        keyframe_tx,
        screen_info_tx,
        screen_info: screen_info_arc,
        cursor_tx,
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new(&web_dir))
        .with_state(state);

    // 监听 0.0.0.0 时打印安全提示（见 docs/04 第 6 节）。
    if bind.starts_with("0.0.0.0") {
        warn!(
            "正在监听所有网卡 (0.0.0.0)，服务暴露于整个局域网。请勿直接暴露到公网，并设置足够复杂的密码。"
        );
    }

    // M4: 根据 TLS 配置决定启动 HTTPS 还是 HTTP
    if tls_enabled {
        let rustls_config = crate::tls::make_rustls_config(&tls_config)?;
        let tls_config = axum_server::tls_rustls::RustlsConfig::from_config(rustls_config);
        let addr: SocketAddr = bind.parse()?;
        info!("HTTPS 服务已启动: https://{}", bind);

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        let listener = TcpListener::bind(&bind).await?;
        info!(
            "HTTP 服务已启动: http://{}（TLS 未启用，仅限开发环境）",
            bind
        );

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
    }

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
    // 输入注入器：认证成功后按需创建（避免未认证连接占用资源）。
    let mut injector: Option<Box<dyn InputInjector>> = None;
    // 视频接收器：认证成功后创建
    let mut video_rx: Option<tokio::sync::broadcast::Receiver<Vec<u8>>> = None;
    // 屏幕信息变化接收器：认证成功后创建
    let mut screen_info_change_rx: Option<tokio::sync::broadcast::Receiver<ScreenInfo>> = None;
    // 光标形状变化接收器：认证成功后创建
    let mut cursor_rx: Option<tokio::sync::broadcast::Receiver<CursorShape>> = None;

    loop {
        // M3: 认证后同时监听客户端消息、视频帧和屏幕信息变化
        let msg = if video_rx.is_some() {
            tokio::select! {
                frame_result = socket.recv() => {
                    match frame_result {
                        Some(Ok(Message::Text(text))) => match parse_client_msg(&text) {
                            Ok(m) => Some(m),
                            Err(e) => {
                                warn!("{}: 消息解析失败，已忽略: {}", peer_addr, e);
                                continue;
                            }
                        },
                        Some(Ok(Message::Close(_))) => {
                            debug!("{}: 收到 Close 帧", peer_addr);
                            break;
                        }
                        Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                        Some(Ok(Message::Binary(_))) => {
                            debug!("{}: 收到二进制帧（暂不处理）", peer_addr);
                            continue;
                        }
                        Some(Err(e)) => {
                            warn!("{}: WebSocket 接收错误: {}", peer_addr, e);
                            break;
                        }
                        None => break,
                    }
                }
                video_frame = video_rx.as_mut().unwrap().recv() => {
                    match video_frame {
                        Ok(frame) => {
                            if let Err(e) = socket.send(Message::Binary(frame)).await {
                                warn!("{}: 发送视频帧失败: {}", peer_addr, e);
                                break;
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            debug!("{}: 视频接收滞后，跳过 {} 帧", peer_addr, n);
                            continue;
                        }
                        Err(e) => {
                            warn!("{}: 视频接收错误: {}", peer_addr, e);
                            break;
                        }
                    }
                }
                screen_info_change = async {
                    match screen_info_change_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match screen_info_change {
                        Ok(new_info) => {
                            info!("{}: 推送屏幕分辨率变化: {}x{}", peer_addr, new_info.width, new_info.height);
                            if let Err(e) = send_msg(
                                &mut socket,
                                &ServerMsg::ScreenInfo {
                                    width: new_info.width,
                                    height: new_info.height,
                                },
                            ).await {
                                warn!("{}: 发送屏幕信息变化失败: {}", peer_addr, e);
                                break;
                            }
                            // 请求关键帧，确保客户端能立即看到新分辨率的画面
                            let _ = state.keyframe_tx.send(());
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // 滞后时可以忽略，因为最新的 screen_info 会在下次更新时收到
                            continue;
                        }
                        Err(e) => {
                            debug!("{}: 屏幕信息变化接收错误: {}", peer_addr, e);
                            continue;
                        }
                    }
                }
                cursor_change = async {
                    match cursor_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match cursor_change {
                        Ok(cursor) => {
                            use base64::{Engine as _, engine::general_purpose};
                            let rgba_base64 = general_purpose::STANDARD.encode(&cursor.rgba);
                            if let Err(e) = send_msg(
                                &mut socket,
                                &ServerMsg::CursorShape {
                                    width: cursor.width,
                                    height: cursor.height,
                                    xhot: cursor.xhot,
                                    yhot: cursor.yhot,
                                    rgba_base64,
                                },
                            ).await {
                                warn!("{}: 发送光标形状失败: {}", peer_addr, e);
                                break;
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // 光标变化滞后可忽略
                            continue;
                        }
                        Err(e) => {
                            debug!("{}: 光标形状接收错误: {}", peer_addr, e);
                            continue;
                        }
                    }
                }
            }
        } else {
            // 未认证：只监听客户端消息
            match socket.recv().await {
                Some(Ok(Message::Text(text))) => match parse_client_msg(&text) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        warn!("{}: 消息解析失败，已忽略: {}", peer_addr, e);
                        continue;
                    }
                },
                Some(Ok(Message::Close(_))) => {
                    debug!("{}: 收到 Close 帧", peer_addr);
                    break;
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Binary(_))) => {
                    debug!("{}: 收到二进制帧（暂不处理）", peer_addr);
                    continue;
                }
                Some(Err(e)) => {
                    warn!("{}: WebSocket 接收错误: {}", peer_addr, e);
                    break;
                }
                None => break,
            }
        };

        let Some(msg) = msg else { continue };

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

                            // 发送 auth_ok
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

                            // M3: 发送屏幕信息
                            let current_screen_info = state.screen_info.read().await;
                            if send_msg(
                                &mut socket,
                                &ServerMsg::ScreenInfo {
                                    width: current_screen_info.width,
                                    height: current_screen_info.height,
                                },
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                            drop(current_screen_info);

                            // M3: 订阅视频流
                            video_rx = Some(state.video_tx.subscribe());
                            info!("{}: 视频流已订阅", peer_addr);

                            // 订阅屏幕信息变化
                            screen_info_change_rx = Some(state.screen_info_tx.subscribe());
                            debug!("{}: 屏幕信息变化已订阅", peer_addr);

                            // 订阅光标形状变化
                            cursor_rx = Some(state.cursor_tx.subscribe());
                            debug!("{}: 光标形状变化已订阅", peer_addr);

                            // 立即请求关键帧，确保新连接能收到 SPS/PPS/IDR。
                            let _ = state.keyframe_tx.send(());

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
                    ClientMsg::Input {
                        event,
                        x,
                        y,
                        button,
                        dx,
                        dy,
                        code,
                        modifiers: _,
                    } => {
                        // 首次输入事件时延迟初始化注入器：失败只 warn，不断连，
                        // 但后续事件就直接跳过（避免每事件重试日志风暴）。
                        if injector.is_none() {
                            match input::create_injector() {
                                Ok(inj) => {
                                    info!("{}: 输入注入器已就绪", peer_addr);
                                    injector = Some(inj);
                                }
                                Err(e) => {
                                    warn!(
                                        "{}: 创建输入注入器失败，本会话将丢弃所有输入: {}",
                                        peer_addr, e
                                    );
                                    // 用兜底标记防止反复重试：下一节用 None 跳过。
                                    // 这里继续 break match，等下一条消息。
                                    continue;
                                }
                            }
                        }
                        if let Some(inj) = injector.as_mut() {
                            let screen_info = state.screen_info.read().await;
                            if let Err(e) = dispatch_input(
                                inj.as_mut(),
                                &event,
                                x,
                                y,
                                &button,
                                dx,
                                dy,
                                &code,
                                screen_info.width as i32,
                                screen_info.height as i32,
                            ) {
                                warn!("{}: 注入输入事件 '{}' 失败: {}", peer_addr, event, e);
                            }
                        }
                    }
                    ClientMsg::Control { action } => {
                        if action == "request_keyframe" {
                            debug!("{}: 收到关键帧请求", peer_addr);
                            let _ = state.keyframe_tx.send(());
                        } else {
                            debug!("{}: 收到未知 control '{}'，忽略", peer_addr, action);
                        }
                    }
                    ClientMsg::ClipboardSet { text } => {
                        handle_clipboard_set(&mut socket, peer_addr, &state, text).await;
                    }
                    ClientMsg::ClipboardGet => {
                        handle_clipboard_get(&mut socket, peer_addr, &state).await;
                    }
                    ClientMsg::SpecialKey { key } => {
                        // 处理浏览器无法捕获的系统快捷键
                        if injector.is_none() {
                            match input::create_injector() {
                                Ok(inj) => {
                                    info!("{}: 输入注入器已就绪", peer_addr);
                                    injector = Some(inj);
                                }
                                Err(e) => {
                                    warn!("{}: 创建输入注入器失败: {}", peer_addr, e);
                                    continue;
                                }
                            }
                        }
                        if let Some(inj) = injector.as_mut() {
                            if let Err(e) = handle_special_key(inj.as_mut(), &key) {
                                warn!("{}: 注入特殊快捷键 '{}' 失败: {}", peer_addr, key, e);
                            } else {
                                debug!("{}: 已注入特殊快捷键: {}", peer_addr, key);
                            }
                        }
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

/// 处理 clipboard_set：把控制端剪切板文本写入被控端系统剪切板。
///
/// 剪切板访问是阻塞的 X11 调用，放到 spawn_blocking 里执行，避免卡住异步运行时。
async fn handle_clipboard_set(
    socket: &mut WebSocket,
    peer_addr: SocketAddr,
    state: &Arc<AppState>,
    text: String,
) {
    let cfg = &state.config.clipboard;
    if !cfg.enabled {
        debug!("{}: 剪切板同步已禁用，忽略 clipboard_set", peer_addr);
        let _ = send_msg(
            socket,
            &ServerMsg::ClipboardError {
                op: "set".to_string(),
                message: "剪切板同步已被服务端禁用".to_string(),
            },
        )
        .await;
        return;
    }

    if text.len() > cfg.max_text_bytes {
        warn!(
            "{}: clipboard_set 文本 {} 字节超过上限 {}，已拒绝",
            peer_addr,
            text.len(),
            cfg.max_text_bytes
        );
        let _ = send_msg(
            socket,
            &ServerMsg::ClipboardError {
                op: "set".to_string(),
                message: format!("文本超过上限 {} 字节", cfg.max_text_bytes),
            },
        )
        .await;
        return;
    }

    // 不打印剪切板内容，只记长度。
    debug!("{}: 写入被控端剪切板（{} 字节）", peer_addr, text.len());
    let result = tokio::task::spawn_blocking(move || clipboard::set_text(&text)).await;

    match result {
        Ok(Ok(())) => { /* 成功无需回复 */ }
        Ok(Err(e)) => {
            warn!("{}: 写入剪切板失败: {}", peer_addr, e);
            let _ = send_msg(
                socket,
                &ServerMsg::ClipboardError {
                    op: "set".to_string(),
                    message: e.to_string(),
                },
            )
            .await;
        }
        Err(e) => {
            warn!("{}: 剪切板写入任务异常: {}", peer_addr, e);
        }
    }
}

/// 处理 clipboard_get：读取被控端系统剪切板文本，回发给控制端。
async fn handle_clipboard_get(
    socket: &mut WebSocket,
    peer_addr: SocketAddr,
    state: &Arc<AppState>,
) {
    let cfg = &state.config.clipboard;
    if !cfg.enabled {
        debug!("{}: 剪切板同步已禁用，忽略 clipboard_get", peer_addr);
        let _ = send_msg(
            socket,
            &ServerMsg::ClipboardError {
                op: "get".to_string(),
                message: "剪切板同步已被服务端禁用".to_string(),
            },
        )
        .await;
        return;
    }

    let max = cfg.max_text_bytes;
    let result = tokio::task::spawn_blocking(clipboard::get_text).await;

    match result {
        Ok(Ok(mut text)) => {
            // 超长时截断到字符边界，避免把超大剪切板整个塞进 WebSocket。
            if text.len() > max {
                warn!(
                    "{}: 被控端剪切板 {} 字节超过上限 {}，已截断",
                    peer_addr,
                    text.len(),
                    max
                );
                truncate_to_char_boundary(&mut text, max);
            }
            debug!("{}: 读取被控端剪切板（{} 字节）", peer_addr, text.len());
            let _ = send_msg(socket, &ServerMsg::ClipboardData { text }).await;
        }
        Ok(Err(e)) => {
            warn!("{}: 读取剪切板失败: {}", peer_addr, e);
            let _ = send_msg(
                socket,
                &ServerMsg::ClipboardError {
                    op: "get".to_string(),
                    message: e.to_string(),
                },
            )
            .await;
        }
        Err(e) => {
            warn!("{}: 剪切板读取任务异常: {}", peer_addr, e);
            let _ = send_msg(
                socket,
                &ServerMsg::ClipboardError {
                    op: "get".to_string(),
                    message: "剪切板读取任务异常".to_string(),
                },
            )
            .await;
        }
    }
}

/// 把字符串截断到不超过 `max` 字节的最近字符边界，保证仍是合法 UTF-8。
fn truncate_to_char_boundary(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// 把协议层 input 事件分派到注入器。
///
/// `event` 决定走哪条路径，多余字段被忽略。坐标 `x`/`y` 是归一化的 0.0~1.0，
/// 在这里换算成像素坐标。
fn dispatch_input(
    injector: &mut dyn InputInjector,
    event: &str,
    x: f64,
    y: f64,
    button: &str,
    dx: i32,
    dy: i32,
    code: &str,
    screen_width: i32,
    screen_height: i32,
) -> Result<()> {
    match event {
        "mouse_move" => {
            let (px, py) = norm_to_pixel(x, y, screen_width, screen_height);
            injector.move_mouse(px, py)
        }
        "mouse_down" | "mouse_up" => {
            let pressed = event == "mouse_down";
            let (px, py) = norm_to_pixel(x, y, screen_width, screen_height);
            // 点击前先挪到目标位置，确保 button down/up 在浏览器看到的位置发生。
            // 鼠标移动失败不应阻塞按键事件本身。
            if let Err(e) = injector.move_mouse(px, py) {
                debug!("点击前 move_mouse 失败（忽略，继续按键）: {}", e);
            }
            let Some(btn) = MouseButton::from_str(button) else {
                debug!("未支持的鼠标按键 '{}'，忽略事件", button);
                return Ok(());
            };
            injector.mouse_button(btn, pressed)
        }
        "wheel" => {
            // 滚轮事件落在 X 指针当前所在窗口，所以滚动前先把指针挪到目标位置，
            // 否则会滚到上一次指针停留的窗口（表现为「不先按一下就没反应」）。
            // 移动失败不应阻塞滚动本身。
            let (px, py) = norm_to_pixel(x, y, screen_width, screen_height);
            if let Err(e) = injector.move_mouse(px, py) {
                debug!("滚动前 move_mouse 失败（忽略，继续滚动）: {}", e);
            }
            injector.scroll(dx, dy)
        }
        "key_down" | "key_up" => {
            let pressed = event == "key_down";
            injector.key(code, pressed)
        }
        other => {
            debug!("未支持的 input 事件 '{}'，忽略", other);
            Ok(())
        }
    }
}

/// 归一化 (0.0~1.0) → 屏幕像素坐标。M3 使用真实屏幕分辨率。
fn norm_to_pixel(x: f64, y: f64, screen_width: i32, screen_height: i32) -> (i32, i32) {
    let nx = x.clamp(0.0, 1.0);
    let ny = y.clamp(0.0, 1.0);
    let px = (nx * f64::from(screen_width)).round() as i32;
    let py = (ny * f64::from(screen_height)).round() as i32;
    (px, py)
}

/// 处理特殊快捷键组合（浏览器无法捕获的系统键）。
///
/// 按下所有修饰键 → 按下目标键 → 释放目标键 → 释放修饰键，模拟用户按下组合键。
fn handle_special_key(injector: &mut dyn InputInjector, key: &str) -> Result<()> {
    match key {
        "alt_f4" => {
            // 模拟 Alt+F4：关闭当前窗口
            injector.key("AltLeft", true)?;
            injector.key("F4", true)?;
            injector.key("F4", false)?;
            injector.key("AltLeft", false)?;
            Ok(())
        }
        "ctrl_alt_del" => {
            // 模拟 Ctrl+Alt+Del：Windows 安全屏幕
            // 注意：在 Windows 上，SendInput 无法模拟 Ctrl+Alt+Del（系统安全限制）
            // 需要驱动级注入或专门的 API（如 SAS generation）
            injector.key("ControlLeft", true)?;
            injector.key("AltLeft", true)?;
            injector.key("Delete", true)?;
            injector.key("Delete", false)?;
            injector.key("AltLeft", false)?;
            injector.key("ControlLeft", false)?;
            Ok(())
        }
        "win_d" => {
            // Windows+D：显示桌面
            injector.key("MetaLeft", true)?;
            injector.key("KeyD", true)?;
            injector.key("KeyD", false)?;
            injector.key("MetaLeft", false)?;
            Ok(())
        }
        "win_l" => {
            // Windows+L：锁定计算机
            injector.key("MetaLeft", true)?;
            injector.key("KeyL", true)?;
            injector.key("KeyL", false)?;
            injector.key("MetaLeft", false)?;
            Ok(())
        }
        other => {
            debug!("未支持的特殊快捷键 '{}'，忽略", other);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_to_pixel_clamps_and_rounds() {
        assert_eq!(norm_to_pixel(0.0, 0.0, 1920, 1080), (0, 0));
        assert_eq!(norm_to_pixel(1.0, 1.0, 1920, 1080), (1920, 1080));
        assert_eq!(norm_to_pixel(0.5, 0.5, 1920, 1080), (960, 540));
        // 越界值被夹紧。
        assert_eq!(norm_to_pixel(-0.5, 2.0, 1920, 1080), (0, 1080));
    }
}
