//! Wayland 输入注入实现
//!
//! ## 方案取舍
//!
//! 「原生 Wayland」下的输入注入没有统一的用户态入口：
//!
//! - `libei` / `xdg-remote-desktop-portal`：官方推荐，但需要新版 xdg-desktop-portal
//!   + libei 服务端；国产化 Wayland（Kylin/UKUI、DDE）目前尚未完整落地。
//! - `uinput`（内核 evdev）：无 compositor 依赖，缺点是需要 root 或 input 组权限，
//!   坐标系是虚拟设备而非「桌面像素」。
//! - `ydotool`：本质上就是 uinput 的用户态守护。
//!
//! 好在国内主流 Wayland 桌面（Kylin/UKUI、Deepin 等）都自动拉起 Xwayland，
//! 并把 Xwayland 的输入事件桥接到 compositor —— 这意味着通过 XTest 注入的键鼠事件
//! 会作用到真实的 Wayland 桌面（我们在麒麟 V10 SP1 UKUI 上通过 `xdotool` 已验证）。
//!
//! 因此本实现的策略是：Wayland 会话下，**优先走 Xwayland 的 XTest 通道**（复用
//! `X11Injector` 的实现），如果 X server 不可用再让上层降级。等 libei / portal
//! 生态在国产化环境铺开后，可在本模块内加入原生 Wayland 后端并做运行时切换。

use super::{InputInjector, MouseButton};
use anyhow::{Context, Result};
use tracing::info;

/// Wayland 输入注入器
///
/// 目前包裹 `X11Injector`，通过 Xwayland/XTest 把事件送到 Wayland compositor。
pub struct WaylandInjector {
    inner: super::x11::X11Injector,
}

impl WaylandInjector {
    /// 初始化 Wayland 输入注入器。
    pub fn new() -> Result<Self> {
        let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
        let x_display = std::env::var("DISPLAY").ok();

        if x_display.as_deref().unwrap_or("").is_empty() {
            anyhow::bail!(
                "Wayland 会话下未检测到 DISPLAY 环境变量。\n\
                 当前实现依赖 Xwayland/XTest 桥接注入事件，请确保已启动 Xwayland；\n\
                 也可在启动前显式导出 DISPLAY=:0 与 XAUTHORITY=$HOME/.Xauthority。"
            );
        }

        info!(
            "Wayland 输入注入：使用 Xwayland/XTest 兼容后端 (WAYLAND_DISPLAY={:?}, DISPLAY={:?})",
            wayland_display.as_deref().unwrap_or("<unset>"),
            x_display.as_deref().unwrap_or("<unset>")
        );

        let inner = super::x11::X11Injector::new()
            .context("Wayland 后端：初始化 X11Injector 失败")?;
        Ok(Self { inner })
    }
}

impl InputInjector for WaylandInjector {
    fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
        self.inner.move_mouse(x, y)
    }

    fn mouse_button(&mut self, btn: MouseButton, pressed: bool) -> Result<()> {
        self.inner.mouse_button(btn, pressed)
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.inner.scroll(dx, dy)
    }

    fn key(&mut self, code: &str, pressed: bool) -> Result<()> {
        self.inner.key(code, pressed)
    }
}
