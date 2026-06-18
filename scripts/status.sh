#!/bin/bash
# RemoteControl 后台服务状态查询脚本

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/remotecontrol-server" ]; then
    WORK_DIR="$SCRIPT_DIR"
else
    WORK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

PID_FILE="$WORK_DIR/.remotecontrol.pid"
LOG_FILE="$WORK_DIR/remotecontrol.log"
PROCESS_PATTERN="remotecontrol-server"

echo "==> RemoteControl 服务状态"
echo ""

if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
        echo "✓ 服务运行中"
        echo "  PID: $PID"
        echo "  启动时间: $(ps -p "$PID" -o lstart= 2>/dev/null || echo '未知')"
        echo "  内存占用: $(ps -p "$PID" -o rss= 2>/dev/null | awk '{printf "%.1f MB", $1/1024}' || echo '未知')"
        echo "  CPU: $(ps -p "$PID" -o %cpu= 2>/dev/null || echo '未知')%"
    else
        echo "✗ 服务未运行（PID 文件存在但进程已退出）"
        echo "  旧 PID: $PID"
    fi
else
    RUNNING_PID=$(pgrep -f "$PROCESS_PATTERN" 2>/dev/null || true)
    if [ -n "$RUNNING_PID" ]; then
        echo "⚠ 服务运行中（但无 PID 文件）"
        echo "  PID: $RUNNING_PID"
        echo "  可能是手动启动的，运行 ./stop-daemon.sh 可停止"
    else
        echo "✗ 服务未运行"
    fi
fi

echo ""
echo "==> 日志文件"
if [ -f "$LOG_FILE" ]; then
    echo "  路径: $LOG_FILE"
    echo "  大小: $(du -h "$LOG_FILE" | cut -f1)"
    echo "  最后 5 行:"
    echo "  ---"
    tail -5 "$LOG_FILE" 2>/dev/null | sed 's/^/  /'
else
    echo "  日志文件不存在"
fi

echo ""
echo "==> 端口监听"
netstat -tlnp 2>/dev/null | grep "$PROCESS_PATTERN\|8443" | head -3 || echo "  未检测到监听端口"
