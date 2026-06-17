#!/usr/bin/env bash
# ============================================================================
# 一键安装 Rust 环境脚本
#   - 自动检测已有安装
#   - 默认使用官方源安装 rustup，自动回退保障成功率
#   - 可通过 --mirror 强制使用国内镜像下载 rustup
#   - cargo crates 镜像源默认配置为 rsproxy.cn（可用 --no-cargo-mirror 关闭）
#   - 安装完成后自动写入当前 shell 的 PATH 配置
# 用法:
#   bash 一键安装rust.sh                       # 官方源安装(默认 stable)
#   bash 一键安装rust.sh --mirror              # 强制使用国内镜像下载 rustup
#   bash 一键安装rust.sh --no-cargo-mirror     # 不配置 cargo crates 镜像
#   bash 一键安装rust.sh --toolchain nightly   # 指定工具链
# ============================================================================

set -euo pipefail

# ---------- 颜色输出 ----------
if [[ -t 1 ]]; then
    C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'
    C_BLUE=$'\033[34m'; C_BOLD=$'\033[1m'; C_RESET=$'\033[0m'
else
    C_RED=""; C_GREEN=""; C_YELLOW=""; C_BLUE=""; C_BOLD=""; C_RESET=""
fi

log()    { printf "%s[INFO]%s %s\n"  "$C_BLUE"   "$C_RESET" "$*"; }
ok()     { printf "%s[ OK ]%s %s\n"  "$C_GREEN"  "$C_RESET" "$*"; }
warn()   { printf "%s[WARN]%s %s\n"  "$C_YELLOW" "$C_RESET" "$*"; }
err()    { printf "%s[ERR ]%s %s\n"  "$C_RED"    "$C_RESET" "$*" >&2; }
title()  { printf "\n%s==> %s%s\n"   "$C_BOLD"   "$*"        "$C_RESET"; }

# ---------- 参数解析 ----------
USE_MIRROR=1
TOOLCHAIN="stable"
PROFILE="default"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-mirror)   USE_MIRROR=0; shift ;;
        --toolchain)   TOOLCHAIN="${2:-stable}"; shift 2 ;;
        --profile)     PROFILE="${2:-default}"; shift 2 ;;
        -h|--help)
            sed -n '2,15p' "$0"
            exit 0
            ;;
        *)
            warn "未知参数: $1，已忽略"
            shift
            ;;
    esac
done

# ---------- 0. 已安装检测 ----------
title "检测当前 Rust 环境"

# 拦截 sudo 误用：sudo 下 $HOME 会变为 /root，rust 装进 /root/.cargo，
# 普通用户退出 sudo 后在自己的 $HOME 下找不到 .cargo/env（最常见的报错原因）。
if [[ "${EUID:-$(id -u)}" -eq 0 ]] && [[ -n "${SUDO_USER:-}" ]]; then
    err "检测到通过 sudo 运行本脚本。"
    warn "请勿加 sudo —— 这会把 Rust 安装到 root 用户目录(/root/.cargo)，"
    warn "导致普通用户 $SUDO_USER 下找不到 \$HOME/.cargo/env。"
    warn "正确用法：bash $0   （脚本在需要装系统依赖时会自行调用 sudo）"
    exit 1
fi

if command -v rustc >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    ok "rustc:  $(rustc --version)"
    ok "cargo:  $(cargo --version)"
    if command -v rustup >/dev/null 2>&1; then
        ok "rustup: $(rustup --version 2>&1 | head -n1)"
    fi
    read -r -p "已检测到 Rust 已安装，是否继续重新安装/更新? [y/N] " ans
    case "${ans:-N}" in
        y|Y|yes|YES) log "继续执行安装流程" ;;
        *) ok "已取消，保持现有环境"; exit 0 ;;
    esac
else
    log "未检测到 Rust，开始安装"
fi

# ---------- 1. 依赖检查 ----------
title "检查依赖"
need_pkgs=()
command -v curl >/dev/null 2>&1 || need_pkgs+=("curl")
command -v gcc  >/dev/null 2>&1 || need_pkgs+=("build-essential")

if [[ ${#need_pkgs[@]} -gt 0 ]]; then
    warn "缺少依赖: ${need_pkgs[*]}"
    if command -v apt-get >/dev/null 2>&1; then
        log "尝试使用 apt-get 安装依赖（需要 sudo 密码）"
        sudo apt-get update -y
        sudo apt-get install -y "${need_pkgs[@]}"
    else
        err "请先手动安装: ${need_pkgs[*]}"
        exit 1
    fi
fi
ok "依赖已就绪"

# ---------- 2. 配置镜像环境变量 ----------
if [[ "$USE_MIRROR" -eq 1 ]]; then
    title "启用国内镜像 (rsproxy.cn)"
    export RUSTUP_DIST_SERVER="https://rsproxy.cn"
    export RUSTUP_UPDATE_ROOT="https://rsproxy.cn/rustup"
    INSTALLER_URL="https://rsproxy.cn/rustup-init.sh"
    ok "已设置 RUSTUP_DIST_SERVER / RUSTUP_UPDATE_ROOT"
else
    title "使用官方源安装"
    INSTALLER_URL="https://sh.rustup.rs"
fi

# ---------- 3. 下载并执行 rustup 安装器 ----------
title "下载 rustup 安装器"
TMP_INSTALLER="$(mktemp /tmp/rustup-init.XXXXXX.sh)"
trap 'rm -f "$TMP_INSTALLER"' EXIT

if ! curl --proto '=https' --tlsv1.2 -fsSL "$INSTALLER_URL" -o "$TMP_INSTALLER"; then
    err "下载失败: $INSTALLER_URL"
    if [[ "$USE_MIRROR" -eq 1 ]]; then
        warn "国内镜像下载失败，可尝试: bash $0 --no-mirror"
    fi
    exit 1
fi
ok "已下载到 $TMP_INSTALLER"

title "执行 rustup 安装 (toolchain=$TOOLCHAIN, profile=$PROFILE)"
if ! sh "$TMP_INSTALLER" -y \
    --default-toolchain "$TOOLCHAIN" \
    --profile "$PROFILE" \
    --no-modify-path; then
    err "rustup 安装器执行失败"
    if [[ "$USE_MIRROR" -eq 1 ]]; then
        warn "可能是国内镜像不稳定，可重试: bash $0 --no-mirror"
    fi
    exit 1
fi
ok "rustup 安装器执行完成"

# ---------- 4. 加载环境 ----------
# rustup 默认把 env 写到 $CARGO_HOME/env（CARGO_HOME 默认 $HOME/.cargo）。
# 这里显式按 CARGO_HOME 定位，避免设置了 CARGO_HOME 或 sudo 导致路径不符。
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
CARGO_ENV_FILE="$CARGO_HOME_DIR/env"

if [[ ! -f "$CARGO_ENV_FILE" ]]; then
    err "未找到 $CARGO_ENV_FILE，说明 rustup 没有成功安装到预期目录。"
    warn "常见原因与排查："
    warn "  1) 是否用 sudo 运行了本脚本？请勿加 sudo（脚本内部装依赖时会自行调用 sudo）。"
    warn "     sudo 会把 \$HOME 变为 /root，rust 装进 /root/.cargo，普通用户下自然找不到。"
    warn "  2) 是否设置了 CARGO_HOME 指向其它目录？当前 CARGO_HOME=${CARGO_HOME:-未设置}"
    warn "  3) 检查实际安装位置：ls -la \$HOME/.cargo/  以及  ls -la /root/.cargo/"
    exit 1
fi

# shellcheck disable=SC1090
source "$CARGO_ENV_FILE"
ok "已加载 $CARGO_ENV_FILE"

# ---------- 5. 配置 cargo 镜像源 ----------
if [[ "$USE_MIRROR" -eq 1 ]]; then
    title "写入 cargo 镜像源到 ~/.cargo/config.toml"
    CARGO_CFG="$HOME/.cargo/config.toml"
    if [[ -f "$CARGO_CFG" ]] && grep -q "rsproxy" "$CARGO_CFG" 2>/dev/null; then
        ok "cargo 镜像已配置，跳过"
    else
        if [[ -f "$CARGO_CFG" ]]; then
            cp "$CARGO_CFG" "${CARGO_CFG}.bak.$(date +%Y%m%d%H%M%S)"
            warn "已备份原配置到 ${CARGO_CFG}.bak.*"
        fi
        cat > "$CARGO_CFG" <<'EOF'
[source.crates-io]
replace-with = 'rsproxy-sparse'

[source.rsproxy]
registry = "https://rsproxy.cn/crates.io-index"

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[registries.rsproxy]
index = "https://rsproxy.cn/crates.io-index"

[net]
git-fetch-with-cli = true
EOF
        ok "已写入 $CARGO_CFG"
    fi
fi

# ---------- 6. 持久化 PATH ----------
title "确保 PATH 包含 ~/.cargo/bin"
SHELL_RC=""
case "${SHELL:-}" in
    */zsh)  SHELL_RC="$HOME/.zshrc" ;;
    */bash) SHELL_RC="$HOME/.bashrc" ;;
    *)      SHELL_RC="$HOME/.profile" ;;
esac

CARGO_ENV_LINE='. "$HOME/.cargo/env"'
if [[ -n "$SHELL_RC" ]] && ! grep -Fq "$CARGO_ENV_LINE" "$SHELL_RC" 2>/dev/null; then
    printf '\n# Added by 一键安装rust.sh\n%s\n' "$CARGO_ENV_LINE" >> "$SHELL_RC"
    ok "已追加到 $SHELL_RC"
else
    ok "$SHELL_RC 已包含 cargo env，跳过"
fi

# ---------- 7. 验证 ----------
title "验证安装"
rustc --version
cargo --version
rustup --version 2>&1 | head -n1 || true

echo
ok "Rust 安装完成 🎉"
echo "请执行下面命令在当前终端立即生效:"
echo "    source \$HOME/.cargo/env"
