#!/bin/bash
# RemoteControl 一键启动脚本（前台）

set -e

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/remotecontrol-server" ]; then
    # 打包模式
    WORK_DIR="$SCRIPT_DIR"
    RUN_CMD="./remotecontrol-server"
else
    # 开发模式
    WORK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    export PATH="$HOME/.cargo/bin:$PATH"
    RUN_CMD="cargo run --release --"
fi

cd "$WORK_DIR"

# X11 环境兜底：本程序依赖 X server（XTest 注入 + MIT-SHM 捕获）。
# 在 Wayland 会话（如 Kylin V11）下，autostart 经 bash -lc 启动时不会继承
# DISPLAY/XAUTHORITY，导致连不上 XWayland 而启动失败。这里显式兜底：
#   1) 环境已有则沿用；
#   2) 否则从同用户的图形会话进程 environ 中探测；
#   3) 仍无则兜底 DISPLAY=:0、XAUTHORITY=~/.Xauthority。
if [ -z "${DISPLAY:-}" ] || [ -z "${XAUTHORITY:-}" ]; then
    for _p in $(pgrep -u "$(id -u)" 2>/dev/null); do
        _env="/proc/$_p/environ"
        [ -r "$_env" ] || continue
        _d=$(cat "$_env" 2>/dev/null | tr '\0' '\n' | grep -m1 '^DISPLAY=' | cut -d= -f2-)
        [ -n "$_d" ] || continue
        [ -z "${DISPLAY:-}" ] && export DISPLAY="$_d"
        if [ -z "${XAUTHORITY:-}" ]; then
            _xa=$(cat "$_env" 2>/dev/null | tr '\0' '\n' | grep -m1 '^XAUTHORITY=' | cut -d= -f2-)
            [ -n "$_xa" ] && export XAUTHORITY="$_xa"
        fi
        break
    done
fi
[ -z "${DISPLAY:-}" ] && export DISPLAY=":0"
[ -z "${XAUTHORITY:-}" ] && [ -f "$HOME/.Xauthority" ] && export XAUTHORITY="$HOME/.Xauthority"
echo "==> 图形环境: DISPLAY=$DISPLAY XAUTHORITY=${XAUTHORITY:-<none>}"

# 打包模式下检查配置文件
if [ -f "$SCRIPT_DIR/remotecontrol-server" ] && [ ! -f config.toml ]; then
    echo "错误: config.toml 不存在，请先执行:"
    echo "  cp config.example.toml config.toml"
    exit 1
fi

PROCESS_PATTERN="remotecontrol-server"
PID_FILE="$WORK_DIR/.remotecontrol.pid"
LOG_FILE="$WORK_DIR/remotecontrol.log"

echo "==> 检查已有进程..."
EXISTING_PID=$(pgrep -f "$PROCESS_PATTERN" 2>/dev/null || true)

if [ -n "$EXISTING_PID" ]; then
    echo "    发现已运行的进程: PID $EXISTING_PID，正在停止..."
    kill "$EXISTING_PID" 2>/dev/null || true
    sleep 2
    kill -0 "$EXISTING_PID" 2>/dev/null && kill -9 "$EXISTING_PID" 2>/dev/null || true
    echo "    已停止旧进程"
fi

rm -f "$PID_FILE"

# 开发模式下编译
if [ ! -f "$SCRIPT_DIR/remotecontrol-server" ]; then
    echo "==> 编译项目..."
    cargo build --release 2>&1 | tail -5
fi

echo "==> 启动 RemoteControl 服务..."
echo "    工作目录: $WORK_DIR"
echo "    访问地址: https://$(hostname -I | awk '{print $1}'):8443"
echo "    按 Ctrl+C 停止服务"
echo ""

exec $RUN_CMD "$@"
