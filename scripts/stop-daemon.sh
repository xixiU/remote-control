#!/bin/bash
# RemoteControl 后台服务停止脚本

set -e

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/remotecontrol-server" ]; then
    WORK_DIR="$SCRIPT_DIR"
else
    WORK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

PID_FILE="$WORK_DIR/.remotecontrol.pid"
PROCESS_PATTERN="remotecontrol-server"

echo "==> 停止 RemoteControl 服务..."

if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
        echo "    停止进程 $PID..."
        kill "$PID" 2>/dev/null || true
        sleep 2
        kill -0 "$PID" 2>/dev/null && kill -9 "$PID" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
fi

REMAINING=$(pgrep -f "$PROCESS_PATTERN" 2>/dev/null || true)
if [ -n "$REMAINING" ]; then
    echo "    清理残留进程: $REMAINING"
    pkill -f "$PROCESS_PATTERN" 2>/dev/null || true
    sleep 1
    pkill -9 -f "$PROCESS_PATTERN" 2>/dev/null || true
fi

echo "✓ 服务已停止"
