#!/bin/bash
# RemoteControl 开机自启安装脚本（非 root / 用户级）
#
# 本程序依赖 X11 图形会话（输入注入 + 屏幕捕获都需要 DISPLAY），
# 采用桌面 autostart：登录图形桌面后自动拉起服务，此时 DISPLAY/XAUTHORITY 已就绪。
# 全部写在 ~/.config/autostart/ 下，不需要 root。
#
# 用法:
#   ./install-autostart.sh          # 安装
#   ./install-autostart.sh --remove # 卸载

set -e

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/remotecontrol-server" ]; then
    # 打包模式：脚本与主程序同目录
    PKG_DIR="$SCRIPT_DIR"
    START_SCRIPT="$PKG_DIR/start-daemon.sh"
else
    # 开发模式：脚本在 scripts/ 下
    PKG_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    START_SCRIPT="$PKG_DIR/scripts/start-daemon.sh"
fi

AUTOSTART_DIR="$HOME/.config/autostart"
DESKTOP_FILE="$AUTOSTART_DIR/remotecontrol.desktop"

if [[ "${1:-}" == "--remove" || "${1:-}" == "-r" ]]; then
    echo "==> 卸载开机自启..."
    rm -f "$DESKTOP_FILE" && echo "✓ 已删除自启项" || echo "  未找到自启项"
    echo "  （已运行的服务不受影响，停止请执行 ./stop-daemon.sh）"
    exit 0
fi

echo "==> 安装开机自启（用户级，无需 root）"

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    echo "✗ 请勿用 root/sudo 运行：开机自启需绑定你的图形桌面会话，必须以普通用户安装。"
    exit 1
fi

if [[ ! -f "$START_SCRIPT" ]]; then
    echo "✗ 未找到 $START_SCRIPT"
    exit 1
fi
chmod +x "$START_SCRIPT"

if [[ ! -f "$PKG_DIR/config.toml" ]]; then
    echo "⚠ config.toml 不存在，建议先: cp config.example.toml config.toml 并改密码"
fi

mkdir -p "$AUTOSTART_DIR"
# Exec 内禁用单引号包裹含 ';' 的命令：UKUI/MATE 会判为非法 Exec 而拒绝执行。
# 统一用双引号包裹整条命令，脚本路径再用转义双引号 \" 括起。
cat > "$DESKTOP_FILE" <<DESKTOP
[Desktop Entry]
Type=Application
Name=RemoteControl Server
Comment=远程控制被控端服务（登录后自动启动）
Exec=bash -lc "sleep 5; exec \\"$START_SCRIPT\\""
Path=$PKG_DIR
Terminal=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=5
DESKTOP
chmod +x "$DESKTOP_FILE" 2>/dev/null || true

echo "✓ 已写入自启项: $DESKTOP_FILE"
echo "  登录图形桌面后约 5 秒自动后台启动；日志: $PKG_DIR/remotecontrol.log"
echo "  立即测试: $START_SCRIPT    卸载自启: $0 --remove"
