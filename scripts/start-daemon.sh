#!/bin/bash
# RemoteControl 后台启动脚本

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

echo "==> 启动服务（后台模式）..."
echo "    工作目录: $WORK_DIR"
echo "    日志: $LOG_FILE"
echo "    访问: https://$(hostname -I | awk '{print $1}'):8443"
echo ""

nohup $RUN_CMD "$@" > "$LOG_FILE" 2>&1 &
SERVER_PID=$!
echo "$SERVER_PID" > "$PID_FILE"
sleep 2

if kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "✓ 服务已启动，PID: $SERVER_PID"
    echo "  查看日志: tail -f $LOG_FILE"
    echo "  停止服务: ./stop-daemon.sh"
else
    echo "✗ 启动失败，请查看日志: $LOG_FILE"
    rm -f "$PID_FILE"
    exit 1
fi
