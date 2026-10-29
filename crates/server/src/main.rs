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
use tracing::{info, warn};
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
    // Windows: 启用 DPI 感知，确保高 DPI 显示器下鼠标坐标准确
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
        };
        // 忽略错误（可能已设置或系统不支持）
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

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

    // 兜底清理上一次运行残留的按下状态（详见 input::release_all_modifiers 文档）。
    //
    // 上次进程被强杀（SIGKILL / 任务管理器结束进程）时，web.rs 的三道兜底
    // 全部来不及执行，残留的按下修饰键留在操作系统里而非进程里 —— 重启服务
    // 也不会自愈。这里在启动时无条件清一遍，是唯一不依赖「上次退出执行过
    // 代码」的手段。
    //
    // 失败只记日志绝不中断启动：清理是附加保险，没有它服务照样能用，
    // 让它把服务拦在门外是本末倒置。
    if let Err(e) = input::release_all_modifiers() {
        warn!("启动时清理残留修饰键失败（不影响服务启动）: {}", e);
    }

    // 注册退出信号处理，在进程结束前释放残留按键（即时生效的那一半兜底）。
    spawn_exit_cleanup();

    // 启动 HTTP 服务
    web::serve(config, authenticator).await?;

    Ok(())
}

/// 监听可捕获的退出信号，在进程真正结束前释放残留修饰键。
///
/// ## 为什么不走 graceful shutdown
///
/// 没有改 `web::serve` 的服务器启动结构：HTTP 分支用 `axum::serve`、HTTPS 分支
/// 用 `axum_server::bind_rustls`，两者的优雅关闭是两套不同 API，都要改一遍。
/// 而我们在退出前要做的事只有「发一遍 keyup」，毫秒级且不依赖任何连接状态，
/// 独立 task 收到信号后直接清理并 `exit` 更简单，也更不容易被将来的改动破坏。
///
/// ## 覆盖范围（务必对照着看，不要高估）
///
/// 能兜住：
/// - Linux：`SIGTERM`（`kill` 默认、systemd stop）、`SIGINT`（Ctrl+C）、
///   `SIGHUP`（SSH 断开 / 终端关闭）
/// - Windows：Ctrl+C、Ctrl+Break、控制台窗口关闭（含任务管理器对带控制台
///   窗口的**应用级**条目点「结束任务」，走 `CTRL_CLOSE_EVENT`，系统给约 5s）
///
/// **兜不住**（操作系统保证，无任何实现手段可绕过）：
/// - Linux `SIGKILL`（`kill -9`）
/// - Windows `TerminateProcess`：任务管理器「详细信息」里选中进程结束任务、
///   结束进程树、`taskkill /F`
/// - Windows 用户注销 / 系统关机：本程序加载了 user32.dll（`SendInput`、
///   `SetProcessDpiAwarenessContext`），而 MS 文档明确，加载 gdi32/user32 的
///   控制台程序**收不到** `CTRL_LOGOFF_EVENT` / `CTRL_SHUTDOWN_EVENT`
///   （操作系统把它当 GUI 程序而非控制台程序）。要覆盖这两个得另建隐藏窗口
///   处理 `WM_QUERYENDSESSION`，成本另一个量级，故未实现。
/// - 断电 / 内核 panic
///
/// 以上兜不住的情况由启动时清理（见 `main`）在**下次启动**时修复。
///
/// ## 本项目自带停止脚本的实际覆盖情况
///
/// - `scripts/stop-daemon.sh`：先 `kill`（SIGTERM）、等 2s 再 `kill -9`，
///   本清理**能生效**（10 个 keyup 远在 2s 内完成）。
/// - `scripts/stop-daemon.ps1`：用 `Stop-Process -Force`，等价于
///   `TerminateProcess`，本清理**不生效**，只能靠下次启动时的清理修复。
///   注意去掉 `-Force` 也无济于事：`Stop-Process` 对无窗口进程仍走
///   TerminateProcess，不会发 `CTRL_CLOSE_EVENT`。要让 Windows 停止脚本
///   走优雅路径，得改用 `GenerateConsoleCtrlEvent` 或 `taskkill`（不带 `/F`）
///   —— 那是停止脚本的改动，未在本次变更范围内。
fn spawn_exit_cleanup() {
    tokio::spawn(async {
        let signal_name = wait_for_exit_signal().await;
        info!("收到退出信号 {}，释放残留按键后退出", signal_name);

        // 这里故意**不用** `spawn_blocking`，尽管注入是阻塞的 X11/SendInput 调用
        // （项目其它地方的阻塞调用都走了 spawn_blocking，见 web.rs 的剪切板处理）。
        // 理由：此刻进程正在退出，没有别的异步工作需要这个 worker 线程，
        // 而 Windows 的 CTRL_CLOSE 只给约 5s 窗口，直接执行少一次任务调度，
        // 不必承担 blocking 线程池繁忙时排队的风险。
        //
        // 清理失败也要继续退出：信号已经到了，赖着不走只会让用户以为进程卡死。
        if let Err(e) = input::release_all_modifiers() {
            warn!("退出前清理残留修饰键失败: {}", e);
        }

        // tracing 的 fmt 订阅器默认写 stdout 且每条记录即时 flush，
        // 不像 BufWriter 那样攒着，所以 exit 前无需额外 flush。
        //
        // 必须显式 exit：捕获信号后操作系统的默认终止行为就不会发生了
        // （Windows 侧 tokio 的 console handler 对 close 事件刻意永不返回，
        // 正是为了避免「返回即被终止」），不自己退就真的挂住不动。
        // 退出码 0：这是用户/系统主动要求的正常退出，不是故障。
        std::process::exit(0);
    });
}

/// 等待任意一个可捕获的退出信号，返回信号名用于日志。
///
/// 注册失败（权限受限等罕见情况）时退化为永久挂起而不是直接退出：
/// 清理能力丢了，但绝不能因此把服务搞停 —— 那比残留按键严重得多。
#[cfg(unix)]
async fn wait_for_exit_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};

    // 三个流都要先建好再 select：在 select 分支里现场创建会在每次
    // 轮询时重建流，丢掉已到达的信号。
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!("注册 SIGTERM 处理失败，退出时将不清理残留按键: {}", e);
            return std::future::pending().await;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            warn!("注册 SIGINT 处理失败，退出时将不清理残留按键: {}", e);
            return std::future::pending().await;
        }
    };
    let mut sighup = match signal(SignalKind::hangup()) {
        Ok(s) => s,
        Err(e) => {
            warn!("注册 SIGHUP 处理失败，退出时将不清理残留按键: {}", e);
            return std::future::pending().await;
        }
    };

    tokio::select! {
        _ = sigterm.recv() => "SIGTERM",
        _ = sigint.recv() => "SIGINT",
        _ = sighup.recv() => "SIGHUP",
    }
}

/// Windows 版：监听 Ctrl+C / Ctrl+Break / 控制台关闭。
///
/// 不监听 `ctrl_logoff` / `ctrl_shutdown`：本程序加载了 user32.dll，那两个
/// 事件根本不会回调（见 [`spawn_exit_cleanup`] 的覆盖范围说明），注册了
/// 只会给人「已覆盖注销关机」的错误印象。
#[cfg(target_os = "windows")]
async fn wait_for_exit_signal() -> &'static str {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close};

    let mut ctrl_c_sig = match ctrl_c() {
        Ok(s) => s,
        Err(e) => {
            warn!("注册 Ctrl+C 处理失败，退出时将不清理残留按键: {}", e);
            return std::future::pending().await;
        }
    };
    let mut ctrl_break_sig = match ctrl_break() {
        Ok(s) => s,
        Err(e) => {
            warn!("注册 Ctrl+Break 处理失败，退出时将不清理残留按键: {}", e);
            return std::future::pending().await;
        }
    };
    let mut ctrl_close_sig = match ctrl_close() {
        Ok(s) => s,
        Err(e) => {
            warn!("注册控制台关闭处理失败，退出时将不清理残留按键: {}", e);
            return std::future::pending().await;
        }
    };

    tokio::select! {
        _ = ctrl_c_sig.recv() => "Ctrl+C",
        _ = ctrl_break_sig.recv() => "Ctrl+Break",
        _ = ctrl_close_sig.recv() => "控制台关闭",
    }
}

/// 其它平台：没有可用的信号机制，永久挂起（等效于不启用本功能）。
/// 这类平台上 `create_injector` 本身也会返回错误，功能整体不可用。
///
/// 这个分支与上面两个必须严格互斥且合起来穷尽，否则会是「重复定义」或
/// 「找不到函数」的编译错误。`unix` 与 `target_os = "windows"` 互斥
/// （Windows 不是 unix），二者之外（wasm 等）才走这里。
#[cfg(not(any(unix, target_os = "windows")))]
async fn wait_for_exit_signal() -> &'static str {
    std::future::pending().await
}
