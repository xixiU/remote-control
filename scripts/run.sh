#!/bin/bash
# RemoteControl 一键启动脚本

set -e

# 项目根目录（脚本所在目录的上一级）
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

# 确保 cargo 在 PATH 中
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> 编译项目..."
cargo build --release 2>&1 | tail -5

echo "==> 启动 RemoteControl 服务..."
echo "    访问地址: http://0.0.0.0:8443"
echo "    按 Ctrl+C 停止服务"
echo ""

exec cargo run --release -- "$@"
