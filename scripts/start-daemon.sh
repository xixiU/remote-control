#!/bin/bash
# RemoteControl 后台启动脚本
# 用法: ./scripts/start-daemon.sh [cargo run 参数]
# 示例: ./scripts/start-daemon.sh --password mypass

set -e

# 项目根目录（脚本所在目录的上一级）
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

# 确保 cargo 在 PATH 中
export PATH="$HOME/.cargo/bin:$PATH"

# 进程识别：通过 cargo run --release 和项目目录匹配
PROCESS_PATTERN="remotecontrol-server"
PID_FILE="$PROJECT_DIR/.remotecontrol.pid"
LOG_FILE="$PROJECT_DIR/remotecontrol.log"

echo "==> 检查已有进程..."
# 查找正在运行的 remotecontrol-server 进程
EXISTING_PID=$(pgrep -f "$PROCESS_PATTERN" 2>/dev/null || true)

if [ -n "$EXISTING_PID" ]; then
    echo "    发现已运行的进程: PID $EXISTING_PID"
    echo "    正在停止..."
    kill "$EXISTING_PID" 2>/dev/null || true
    sleep 2

    # 如果还活着，强制杀死
    if kill -0 "$EXISTING_PID" 2>/dev/null; then
        echo "    进程未响应，强制终止..."
        kill -9 "$EXISTING_PID" 2>/dev/null || true
        sleep 1
    fi
    echo "    已停止旧进程"
fi

# 清理旧 PID 文件
rm -f "$PID_FILE"

echo "==> 编译项目..."
cargo build --release 2>&1 | tail -5

echo "==> 启动 RemoteControl 服务（后台模式）..."
echo "    日志文件: $LOG_FILE"
echo "    访问地址: https://0.0.0.0:8443 (TLS) 或 http://0.0.0.0:8443 (若配置关闭 TLS)"
echo ""

# 后台启动并记录 PID
nohup cargo run --release -- "$@" > "$LOG_FILE" 2>&1 &
SERVER_PID=$!

# 保存 PID
echo "$SERVER_PID" > "$PID_FILE"

# 等待一下确认启动成功
sleep 2

if kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "✓ 服务已启动，PID: $SERVER_PID"
    echo "  查看日志: tail -f $LOG_FILE"
    echo "  停止服务: kill $SERVER_PID 或 ./scripts/stop-daemon.sh"
else
    echo "✗ 服务启动失败，请检查日志: $LOG_FILE"
    rm -f "$PID_FILE"
    exit 1
fi
