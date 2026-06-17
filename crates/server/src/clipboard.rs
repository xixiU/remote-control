//! 系统剪切板访问（被控端）
//!
//! 用 arboard 屏蔽平台差异。X11 下 arboard 内部会起一个后台线程托管
//! selection ownership，确保 `set_text` 写入的内容在本进程存活期间持续可粘贴。
//!
//! 这里把 `Clipboard` 实例放在 `OnceLock<Mutex<...>>` 中：
//! - 启动阶段不建实例，避免没有 X11/Wayland 时直接卡住进程；
//! - 首次调用 `get_text`/`set_text` 时才连接 X server；
//! - 失败不缓存，下次重试。

use anyhow::{Context, Result};
use std::sync::{Mutex, OnceLock};

use arboard::Clipboard;

/// 全局共享的剪切板句柄。lazy 初始化，失败不缓存。
static CLIPBOARD: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<Clipboard>> {
    CLIPBOARD.get_or_init(|| Mutex::new(None))
}

/// 在锁内拿到一个可用的 Clipboard 实例：没有就建一个；建失败就把错误冒上去。
fn with_clipboard<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&mut Clipboard) -> Result<T>,
{
    let mut guard = slot()
        .lock()
        .map_err(|e| anyhow::anyhow!("剪切板锁已中毒: {}", e))?;

    if guard.is_none() {
        let cb = Clipboard::new().context("无法初始化系统剪切板（检查 X11 显示是否可用）")?;
        *guard = Some(cb);
    }

    let cb = guard.as_mut().expect("刚刚初始化过");
    let result = f(cb);

    // 任何错误都把实例丢掉，让下次调用重连。X11 selection 偶发掉线时这能自愈。
    if result.is_err() {
        *guard = None;
    }
    result
}

/// 读取系统剪切板的文本。
///
/// 剪切板里是图片或非文本时 arboard 返回 `ContentNotAvailable`，
/// 这里转成 `Ok(String::new())`，让上层把它当作"空"而不是错误。
pub fn get_text() -> Result<String> {
    with_clipboard(|cb| match cb.get_text() {
        Ok(s) => Ok(s),
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(e) => Err(anyhow::anyhow!("读取剪切板失败: {}", e)),
    })
}

/// 把文本写入系统剪切板。
pub fn set_text(text: &str) -> Result<()> {
    with_clipboard(|cb| {
        cb.set_text(text.to_string())
            .map_err(|e| anyhow::anyhow!("写入剪切板失败: {}", e))
    })
}
