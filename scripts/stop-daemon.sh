#!/bin/bash
# RemoteControl 后台服务停止脚本

set -e

# 项目根目录
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PID_FILE="$PROJECT_DIR/.remotecontrol.pid"
PROCESS_PATTERN="remotecontrol-server"

echo "==> 停止 RemoteControl 服务..."

# 尝试从 PID 文件读取
if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
        echo "    停止进程 $PID（从 PID 文件）..."
        kill "$PID" 2>/dev/null || true
        sleep 2

        if kill -0 "$PID" 2>/dev/null; then
            echo "    进程未响应，强制终止..."
            kill -9 "$PID" 2>/dev/null || true
        fi
    fi
    rm -f "$PID_FILE"
fi

# 兜底：通过进程名查找
REMAINING=$(pgrep -f "$PROCESS_PATTERN" 2>/dev/null || true)
if [ -n "$REMAINING" ]; then
    echo "    发现残留进程: $REMAINING，正在清理..."
    pkill -f "$PROCESS_PATTERN" 2>/dev/null || true
    sleep 1
    pkill -9 -f "$PROCESS_PATTERN" 2>/dev/null || true
fi

echo "✓ 服务已停止"
