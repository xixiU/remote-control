#!/bin/bash
# RemoteControl 开机自启安装脚本（非 root / 用户级）
#
# 本程序依赖 X11 图形会话（输入注入 XTest + 屏幕捕获都需要 DISPLAY），
# 因此采用「桌面 autostart」方式：在用户登录图形桌面后自动拉起服务，
# 此时 DISPLAY / XAUTHORITY 已由会话准备好，程序可直接使用。
#
# 不使用 root，不写 /etc，全部在 ~/.config/autostart/ 下。
#
# 用法:
#   ./scripts/install-autostart.sh          # 安装开机自启
#   ./scripts/install-autostart.sh --remove # 卸载开机自启

set -e

# 项目根目录（脚本所在目录的上一级）
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
START_SCRIPT="$PROJECT_DIR/scripts/start-daemon.sh"

AUTOSTART_DIR="$HOME/.config/autostart"
DESKTOP_FILE="$AUTOSTART_DIR/remotecontrol.desktop"

# ---------- 卸载 ----------
if [[ "${1:-}" == "--remove" || "${1:-}" == "-r" ]]; then
    echo "==> 卸载 RemoteControl 开机自启..."
    if [[ -f "$DESKTOP_FILE" ]]; then
        rm -f "$DESKTOP_FILE"
        echo "✓ 已删除 $DESKTOP_FILE"
    else
        echo "  未找到自启项，无需卸载"
    fi
    echo "  （已运行的服务不受影响，如需停止请执行 ./scripts/stop-daemon.sh）"
    exit 0
fi

# ---------- 安装前检查 ----------
echo "==> 安装 RemoteControl 开机自启（用户级，无需 root）"

# 拦截 root：root 下没有用户图形会话，autostart 无意义
if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    echo "✗ 请勿用 root / sudo 运行本脚本。"
    echo "  开机自启需要绑定你的图形桌面会话，必须以普通用户安装。"
    exit 1
fi

if [[ ! -f "$START_SCRIPT" ]]; then
    echo "✗ 未找到启动脚本: $START_SCRIPT"
    exit 1
fi

# 确保启动脚本可执行
chmod +x "$START_SCRIPT"

# 提醒配置文件
if [[ ! -f "$PROJECT_DIR/config.toml" ]]; then
    echo "⚠ 注意: $PROJECT_DIR/config.toml 不存在"
    echo "  服务会以默认配置启动。建议先创建配置并设置密码:"
    echo "    cp $PROJECT_DIR/config.example.toml $PROJECT_DIR/config.toml"
fi

# ---------- 写入 autostart desktop 入口 ----------
mkdir -p "$AUTOSTART_DIR"

# Exec 用 bash -lc 加载登录环境（PATH 等），并显式等待 X 就绪。
cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=RemoteControl Server
Comment=远程控制被控端服务（登录后自动启动）
Exec=bash -lc 'sleep 5; "$START_SCRIPT"'
Path=$PROJECT_DIR
Terminal=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=5
EOF

chmod +x "$DESKTOP_FILE" 2>/dev/null || true

echo "✓ 已写入自启项: $DESKTOP_FILE"
echo ""
echo "=========================================="
echo "✓ 开机自启安装完成"
echo ""
echo "工作方式: 你登录图形桌面后约 5 秒，自动后台启动服务。"
echo "          日志: $PROJECT_DIR/remotecontrol.log"
echo ""
echo "立即测试（无需重启）:"
echo "    $START_SCRIPT"
echo ""
echo "卸载自启:"
echo "    $0 --remove"
echo "=========================================="