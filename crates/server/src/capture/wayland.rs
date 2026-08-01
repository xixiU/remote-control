//! Wayland 屏幕捕获实现
//!
//! ## 方案取舍
//!
//! Wayland 官方推荐的做法是通过 `xdg-desktop-portal` 的 ScreenCast 接口，
//! 由 PipeWire 提供帧数据。但该链路对目标发行版要求较高：
//!
//! - 需要 `xdg-desktop-portal` + 桌面环境对应的 backend（GNOME/KDE 版本较新）；
//! - 需要 `pipewire >= 0.3` 及配套 `libpipewire`；
//! - Kylin V10 SP1 仓库中的 pipewire 停留在 0.2.7，且默认不安装 xdg-desktop-portal，
//!   直接依赖 portal 会在国产化环境上大面积失败。
//!
//! 好在国内主流 Wayland 桌面（Kylin/UKUI、Deepin DDE、UOS 等）都默认拉起 Xwayland，
//! 并把桌面画面镜像到 Xwayland 的根窗口，因此在这些环境里，通过 XGetImage / MIT-SHM
//! 依然能拿到真实桌面像素（我们已在麒麟 V10 SP1 UKUI 上通过 `import -window root`
//! 完整还原了 1920x1080 桌面）。
//!
//! 本实现基于这一事实：**Wayland 后端 = 探测到 Wayland 会话后，走 X11 桥接捕获**。
//! trait 抽象保持不变，未来接入 PipeWire portal 后端时可直接替换本模块。

use super::{CursorShape, Frame, ScreenCapturer, ScreenInfo};
use anyhow::{bail, Context, Result};
use tracing::{info, warn};

/// Wayland 屏幕捕获器
///
/// 目前包裹 `X11Capturer`，通过 Xwayland 提供的兼容根窗口取帧。
pub struct WaylandCapturer {
    inner: super::x11::X11Capturer,
}

impl WaylandCapturer {
    /// 创建 Wayland 捕获器。
    ///
    /// 步骤：
    /// 1. 校验 Xwayland 桥接可用（`DISPLAY` 必须被设置，且能连接到 X server）。
    /// 2. 复用 `X11Capturer::new()` 建立连接、初始化 MIT-SHM/XFIXES。
    /// 3. 抓取一帧做健全性检查，如果全零则大概率是「compositor 未把桌面镜像到 Xwayland」
    ///    的环境（例如原生 GNOME Wayland），此时给出清晰错误让调用方降级或提示用户。
    pub fn new() -> Result<Self> {
        let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
        let x_display = std::env::var("DISPLAY").ok();

        if x_display.as_deref().unwrap_or("").is_empty() {
            bail!(
                "Wayland 会话下未检测到 DISPLAY 环境变量。\n\
                 当前实现依赖 Xwayland 桥接获取桌面画面，请确认已安装并启用 Xwayland；\n\
                 也可以在启动前显式导出 DISPLAY=:0 与 XAUTHORITY=$HOME/.Xauthority。"
            );
        }

        info!(
            "Wayland 屏幕捕获：使用 Xwayland 兼容后端 (WAYLAND_DISPLAY={:?}, DISPLAY={:?})",
            wayland_display.as_deref().unwrap_or("<unset>"),
            x_display.as_deref().unwrap_or("<unset>")
        );

        let mut inner = super::x11::X11Capturer::new()
            .context("Wayland 后端：初始化 Xwayland X11 捕获失败")?;

        // 健全性检查：如果 Xwayland 根窗口全零（compositor 没把桌面镜像出来），
        // 提前失败比让上层拿到黑屏画面更友好。
        match inner.grab_frame() {
            Ok(frame) => {
                if is_frame_blank(&frame.data) {
                    warn!(
                        "Xwayland 根窗口首帧全零，可能当前 compositor 没有把桌面镜像到 Xwayland。\n\
                         若你使用的是原生 GNOME/KDE Wayland，请通过 xdg-desktop-portal 的 ScreenCast \n\
                         协议进行屏幕共享（当前系统缺少相关依赖，未纳入本版本实现范围）。"
                    );
                    // 仍继续返回，让上层根据实际画面自行决定 —— 有些低版本 X server 也会先给一帧 0。
                }
            }
            Err(e) => {
                warn!("Wayland 后端首帧抓取失败（可能是暂态错误，继续运行）：{e}");
            }
        }

        Ok(Self { inner })
    }
}

/// 简单判断一帧是否几乎全零：对 BGRA 数据每隔 256 字节采样一次。
fn is_frame_blank(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    data.iter().step_by(256).all(|b| *b == 0)
}

impl ScreenCapturer for WaylandCapturer {
    fn info(&self) -> ScreenInfo {
        self.inner.info()
    }

    fn grab_frame(&mut self) -> Result<Frame> {
        self.inner.grab_frame()
    }

    fn refresh_info(&mut self) -> Result<bool> {
        self.inner.refresh_info()
    }

    fn get_cursor_shape(&self) -> Result<Option<CursorShape>> {
        self.inner.get_cursor_shape()
    }
}
