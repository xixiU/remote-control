//! 输入注入 - 平台抽象 trait + 各平台实现
//! M2 实现：定义 trait 与工厂；x11 子模块为 Linux+x11 提供 enigo 实现。

use anyhow::Result;
use tracing::{info, warn};

pub mod keymap;

/// 鼠标按键
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    /// 从协议字符串（"left"/"right"/"middle"）解析。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            _ => None,
        }
    }
}

/// 输入注入器 trait
///
/// 实现需要 `Send`，便于跨线程使用（一个会话/连接持有一个注入器）。
pub trait InputInjector: Send {
    /// 移动鼠标到屏幕绝对像素坐标。
    fn move_mouse(&mut self, x: i32, y: i32) -> Result<()>;

    /// 鼠标按键按下/释放。
    fn mouse_button(&mut self, btn: MouseButton, pressed: bool) -> Result<()>;

    /// 滚轮：dx 横向，dy 纵向。单位是浏览器 wheel 事件的 deltaX/deltaY。
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()>;

    /// 键盘按键按下/释放。`code` 取自浏览器 `KeyboardEvent.code`。
    /// 实现内部应通过 [`keymap::code_to_key`] 映射；映射不到时记录日志并返回 Ok。
    fn key(&mut self, code: &str, pressed: bool) -> Result<()>;
}

/// 工厂函数：创建当前平台的输入注入器。
///
/// - Windows：返回 `WindowsInjector`（基于 enigo + SendInput）。
/// - Linux Wayland 会话（feature `wayland`）：返回 `WaylandInjector`（当前实现走 Xwayland/XTest 桥接）。
/// - Linux X11 会话（feature `x11`）：返回 `X11Injector`（基于 enigo + x11rb / XTest）。
/// - 其它平台：暂未实现，返回错误。
pub fn create_injector() -> Result<Box<dyn InputInjector>> {
    #[cfg(target_os = "windows")]
    {
        let injector = windows::WindowsInjector::new()?;
        return Ok(Box::new(injector));
    }

    #[cfg(all(target_os = "linux", feature = "wayland"))]
    {
        if is_wayland_session() {
            let injector = wayland::WaylandInjector::new()?;
            return Ok(Box::new(injector));
        }
    }

    #[cfg(all(target_os = "linux", any(feature = "x11", feature = "wayland")))]
    {
        let injector = x11::X11Injector::new()?;
        return Ok(Box::new(injector));
    }

    #[cfg(not(any(
        target_os = "windows",
        all(target_os = "linux", any(feature = "x11", feature = "wayland")),
    )))]
    {
        anyhow::bail!("当前平台尚未实现输入注入");
    }
}

/// 所有修饰键的 `KeyboardEvent.code`，用于无条件释放。
///
/// 左右键都要列出：注入是按具体 code 做的，而残留的可能是任意一侧。
/// 与 `web.rs` 的 `MODIFIER_GROUPS` 覆盖同一批键，但这里不需要分组，
/// 因为是无条件全部释放，不做「按浏览器状态比对」。
///
/// 故意**不含** CapsLock / NumLock / ScrollLock：那三个是 toggle 状态而非
/// 按下状态，发 keyup 对它们毫无意义，还可能被某些实现当成一次按键而翻转
/// 大小写锁定 —— 那是在清理路径上制造新问题。
///
/// 注意这 10 个 code 经 [`keymap::code_to_key`] 映射后只落到 6 个不同的
/// `enigo::Key`：`AltLeft`/`AltRight` 都是 `Key::Alt`，
/// `MetaLeft`/`MetaRight`/`OSLeft`/`OSRight` 都是 `Key::Meta`。
/// 也就是说实际会对 Alt 发 2 次、Meta 发 4 次重复 keyup。保持原样不去重：
/// 重复释放同样是 no-op，而按 code 列全能在 keymap 将来细分左右键
/// （如 `Key::LAlt`/`Key::RAlt`）时自动受益，不需要回来改这张表。
const ALL_MODIFIER_CODES: &[&str] = &[
    "ShiftLeft",
    "ShiftRight",
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
    "OSLeft",
    "OSRight",
];

/// 无条件释放所有修饰键（Alt / Ctrl / Shift / Meta 的左右键）。
///
/// ## 为什么需要这个
///
/// 远程控制时浏览器可能永久丢掉 keyup（典型是按住 Alt 再按 Tab 切窗口，
/// Alt+Tab 被操作系统吞掉，页面只收到 blur），被控端残留一个按下的 Alt，
/// 之后每个普通键都变成 Alt+X 被菜单吃掉，用户看到的是「键盘被锁定」。
/// `web.rs` 的 `PressedKeys` 已经有三道兜底（按事件纠正、前端失焦主动释放、
/// 连接结束时释放），但它们都依赖**本进程还活着并执行到那段代码**。
///
/// 进程被强杀（SIGKILL / TerminateProcess）时三道兜底全部失效：残留的按下
/// 状态留在操作系统里而不是进程里，重启服务也不会自愈，必须有人到被控端
/// 机器前手动敲一下那个键 —— 无人值守的远程机器就彻底没救了。
///
/// 这个函数在两个时机调用，正是为了补上那个缺口：
/// - **进程启动时**（见 `main.rs`）：兜住上一次的强杀，不依赖上次退出执行过任何代码；
/// - **收到可捕获的退出信号时**（见 `main.rs` 的信号 task）：即时生效。
///
/// ## 幂等性
///
/// 释放一个本来没按下的键是安全的 no-op：X11 走 XTest `fake_input`
/// KEY_RELEASE，对未按下的键 X server 不产生任何状态变化；Windows 走
/// SendInput `KEYEVENTF_KEYUP`，对未按下的键同样不生成有效按键消息。
/// 所以这里无需先查询真实键盘状态，直接全部发一遍即可。
///
/// ## 已知行为（不是 bug，请勿「修复」）
///
/// 启动时无条件清理会释放**被控端本地用户当下正按着的修饰键** —— 比如他正
/// 按着 Shift 打字，服务一启动就把 Shift 放了。这是刻意接受的权衡：
/// 后果是他松手重按一次即恢复（我们只发 keyup 不发 keydown，不会卡死），
/// 而它要修的是「键盘卡死且需人到现场」。本工程的被控端典型场景是无人值守
/// （开机自启 / 远程机器），撞上这个的概率很低。
/// 没有做成配置项：多一个开关就多一处要解释、测试、维护的地方，收益不对等。
///
/// ## 错误处理
///
/// 整体失败（创建注入器失败，如无 DISPLAY）返回 `Err`，由调用方决定是
/// 只记日志还是中断 —— 启动路径上必须只记日志，不能让服务起不来。
/// 单个键释放失败不中断：剩下的键必须继续释放，否则就是在兜底路径上
/// 制造新的卡死。
pub fn release_all_modifiers() -> Result<()> {
    let mut injector = create_injector()?;
    let mut failed = 0usize;
    // 解引用成 &str：迭代 &[&str] 得到的是 &&str，虽能靠 deref 强转匹配
    // key(&str) 但不如显式写清楚。
    for &code in ALL_MODIFIER_CODES {
        if let Err(e) = injector.key(code, false) {
            warn!("释放修饰键 '{}' 失败: {}", code, e);
            failed += 1;
        }
    }
    if failed > 0 {
        warn!(
            "修饰键清理完成，但有 {}/{} 个键释放失败",
            failed,
            ALL_MODIFIER_CODES.len()
        );
    } else {
        info!("已释放全部 {} 个修饰键", ALL_MODIFIER_CODES.len());
    }
    Ok(())
}

/// 判断当前是否运行在 Wayland 会话中。
///
/// 判定顺序：`WAYLAND_DISPLAY` 非空 > `XDG_SESSION_TYPE == "wayland"`。
#[cfg(all(target_os = "linux", feature = "wayland"))]
fn is_wayland_session() -> bool {
    if std::env::var("WAYLAND_DISPLAY").ok().filter(|v| !v.is_empty()).is_some() {
        return true;
    }
    matches!(std::env::var("XDG_SESSION_TYPE").as_deref(), Ok("wayland"))
}

#[cfg(all(target_os = "linux", any(feature = "x11", feature = "wayland")))]
mod x11;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_parses_protocol_strings() {
        assert_eq!(MouseButton::from_str("left"), Some(MouseButton::Left));
        assert_eq!(MouseButton::from_str("right"), Some(MouseButton::Right));
        assert_eq!(MouseButton::from_str("middle"), Some(MouseButton::Middle));
        assert_eq!(MouseButton::from_str("Left"), None);
        assert_eq!(MouseButton::from_str(""), None);
    }
}
