#!/bin/bash
# RemoteControl 构建脚本
# 用法:
#   ./scripts/build.sh              # release 构建
#   ./scripts/build.sh --debug      # debug 构建
#   ./scripts/build.sh --test       # 构建 + 跑测试
#   ./scripts/build.sh --clean      # 先清理再构建

set -e

# 项目根目录（脚本所在目录的上一级）
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

# 确保 cargo 在 PATH 中
export PATH="$HOME/.cargo/bin:$PATH"

# 解析参数
PROFILE="release"
RUN_TESTS=0
DO_CLEAN=0
for arg in "$@"; do
    case "$arg" in
        --debug)   PROFILE="debug" ;;
        --release) PROFILE="release" ;;
        --test)    RUN_TESTS=1 ;;
        --clean)   DO_CLEAN=1 ;;
        -h|--help)
            sed -n '2,8p' "$0"
            exit 0
            ;;
        *)
            echo "未知参数: $arg" >&2
            exit 1
            ;;
    esac
done

# 检查 cargo
if ! command -v cargo >/dev/null 2>&1; then
    echo "错误: 未找到 cargo，请先运行 scripts/一键安装rust.sh" >&2
    exit 1
fi

echo "==> Rust 版本: $(rustc --version)"

if [ "$DO_CLEAN" -eq 1 ]; then
    echo "==> 清理 target 目录..."
    cargo clean
fi

# 构建
if [ "$PROFILE" = "release" ]; then
    echo "==> Release 构建..."
    cargo build --release
    BIN_PATH="$PROJECT_DIR/target/release/remotecontrol-server"
else
    echo "==> Debug 构建..."
    cargo build
    BIN_PATH="$PROJECT_DIR/target/debug/remotecontrol-server"
fi

# 测试
if [ "$RUN_TESTS" -eq 1 ]; then
    echo "==> 运行单元测试..."
    cargo test
fi

echo ""
echo "==> 构建完成"
echo "    二进制: $BIN_PATH"
if [ -f "$BIN_PATH" ]; then
    echo "    大小:   $(du -h "$BIN_PATH" | cut -f1)"
fi
echo "    启动:   ./scripts/run.sh"
