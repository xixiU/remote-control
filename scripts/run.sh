#!/bin/bash
# RemoteControl 一键启动脚本（前台）

set -e

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/remotecontrol-server" ]; then
    # 打包模式：主程序与脚本同目录，以此为工作目录
    WORK_DIR="$SCRIPT_DIR"
    RUN_CMD="./remotecontrol-server"
else
    # 开发模式：脚本在 scripts/ 下，主程序在 target/release/
    WORK_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    export PATH="$HOME/.cargo/bin:$PATH"
    RUN_CMD="cargo run --release --"
fi

cd "$WORK_DIR"

# 打包模式下检查配置文件
if [ -f "$SCRIPT_DIR/remotecontrol-server" ] && [ ! -f config.toml ]; then
    echo "错误: config.toml 不存在"
    echo "请先复制配置模板:"
    echo "  cp config.example.toml config.toml"
    echo "然后修改其中的密码等配置"
    exit 1
fi

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
