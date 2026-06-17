#!/bin/bash
# RemoteControl 打包脚本
# 生成可分发的二进制包，包含可执行文件、静态资源、配置示例、运行脚本

set -e

# 项目根目录
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

# 确保 cargo 在 PATH 中
export PATH="$HOME/.cargo/bin:$PATH"

# 打包输出目录
PACKAGE_NAME="remotecontrol-$(uname -m)-$(date +%Y%m%d-%H%M%S)"
PACKAGE_DIR="dist/$PACKAGE_NAME"

echo "==> RemoteControl 打包脚本"
echo "    目标: $PACKAGE_DIR"
echo ""

# 清理旧的打包目录
rm -rf "dist/$PACKAGE_NAME"
mkdir -p "$PACKAGE_DIR"

echo "==> 1. 编译 release 二进制..."
cargo build --release 2>&1 | tail -5
echo "✓ 编译完成"
echo ""

echo "==> 2. 复制二进制文件..."
cp target/release/remotecontrol-server "$PACKAGE_DIR/"
strip "$PACKAGE_DIR/remotecontrol-server" 2>/dev/null || true  # 去除调试符号减小体积
echo "✓ 二进制: $(du -h "$PACKAGE_DIR/remotecontrol-server" | cut -f1)"
echo ""

echo "==> 3. 复制静态资源..."
cp -r web "$PACKAGE_DIR/"
echo "✓ web/ 目录已复制"
echo ""

echo "==> 4. 生成配置文件示例..."
cat > "$PACKAGE_DIR/config.example.toml" <<'EOF'
# RemoteControl 配置文件
# 使用: cp config.example.toml config.toml 并按需修改

bind = "0.0.0.0:8443"
password = "请修改为复杂密码"

[video]
fps = 30
bitrate = 4000000
codec = "h264"

[security]
max_auth_failures = 5
auth_ban_seconds = 300
session_idle_timeout = 300
exclusive_control = false

[tls]
enabled = true
# 留空则自动生成自签名证书并缓存到 .cache/
# cert = "cert.pem"
# key = "key.pem"
EOF
echo "✓ config.example.toml"
echo ""

echo "==> 5. 生成启动脚本..."
cat > "$PACKAGE_DIR/start.sh" <<'EOF'
#!/bin/bash
# RemoteControl 启动脚本

set -e

# 切换到脚本所在目录（确保相对路径正确）
cd "$(dirname "$0")"

# 检查配置文件
if [ ! -f config.toml ]; then
    echo "错误: config.toml 不存在"
    echo "请先复制配置模板:"
    echo "  cp config.example.toml config.toml"
    echo "然后修改其中的密码等配置"
    exit 1
fi

echo "==> 启动 RemoteControl 服务..."
echo "    配置文件: config.toml"
echo "    访问地址: https://$(hostname -I | awk '{print $1}'):8443"
echo "    按 Ctrl+C 停止"
echo ""

exec ./remotecontrol-server --config config.toml "$@"
EOF
chmod +x "$PACKAGE_DIR/start.sh"
echo "✓ start.sh（前台运行）"

cat > "$PACKAGE_DIR/start-daemon.sh" <<'EOF'
#!/bin/bash
# RemoteControl 后台启动脚本

set -e
cd "$(dirname "$0")"

# 检查配置文件
if [ ! -f config.toml ]; then
    echo "错误: config.toml 不存在，请先执行:"
    echo "  cp config.example.toml config.toml"
    exit 1
fi

PROCESS_PATTERN="remotecontrol-server"
PID_FILE=".remotecontrol.pid"
LOG_FILE="remotecontrol.log"

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

echo "==> 启动服务（后台模式）..."
echo "    日志: $LOG_FILE"
echo "    访问: https://$(hostname -I | awk '{print $1}'):8443"
echo ""

nohup ./remotecontrol-server --config config.toml "$@" > "$LOG_FILE" 2>&1 &
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
EOF
chmod +x "$PACKAGE_DIR/start-daemon.sh"
echo "✓ start-daemon.sh（后台运行）"

cat > "$PACKAGE_DIR/stop-daemon.sh" <<'EOF'
#!/bin/bash
# RemoteControl 停止脚本

set -e
cd "$(dirname "$0")"

PID_FILE=".remotecontrol.pid"
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
EOF
chmod +x "$PACKAGE_DIR/stop-daemon.sh"
echo "✓ stop-daemon.sh（停止服务）"
echo ""

echo "==> 6. 生成 README..."
cat > "$PACKAGE_DIR/README.md" <<'EOF'
# RemoteControl 分发包

RemoteControl 是一个基于 WebSocket + H.264 + WebCodecs 的远程桌面控制工具。

## 系统要求

- Linux x86_64（X11 环境，Wayland 尚不支持）
- XTest 扩展（X11 输入注入）
- 现代浏览器（支持 WebCodecs，如 Chrome/Edge 103+）

## 快速开始

### 1. 配置
```bash
# 复制配置模板并修改密码
cp config.example.toml config.toml
nano config.toml  # 修改 password 字段
```

### 2. 启动服务

**前台运行（调试用）：**
```bash
./start.sh
```

**后台运行（推荐）：**
```bash
./start-daemon.sh
# 查看日志
tail -f remotecontrol.log
# 停止服务
./stop-daemon.sh
```

### 3. 访问控制端

浏览器打开 `https://<服务器IP>:8443`

⚠️ 首次访问会提示证书不受信任（自签名证书），点击「继续访问」或「高级 → 继续」。

输入密码后即可看到被控端画面并操作。

## 配置说明

- **bind**: 监听地址，默认 `0.0.0.0:8443`（局域网可访问）
- **password**: 访问密码，**务必修改**
- **video.fps**: 视频帧率，默认 30（性能不足可降到 10-20）
- **video.bitrate**: 视频码率，默认 4Mbps
- **tls.enabled**: 是否启用 HTTPS，默认 `true`（推荐保持）

## TLS 证书

默认自动生成自签名证书并缓存到 `.cache/` 目录。若需自定义证书：
```toml
[tls]
enabled = true
cert = "your-cert.pem"
key = "your-key.pem"
```

## 故障排查

### 服务启动失败
```bash
# 查看详细日志
cat remotecontrol.log
```

### 请勿用 sudo 运行
本程序不需要 root 权限，**请用普通用户运行**（输入注入依赖当前用户的 X11 会话）。
用 sudo 运行会引发两类问题：
- 输入注入失败：sudo 环境下 `$DISPLAY` / `XAUTHORITY` 与你的桌面会话不一致，
  连不上 X server，鼠标键盘无法注入。
- 路径错乱：证书缓存 `.cache/`、`config.toml`、日志会按 root 的工作目录/权限生成，
  普通用户后续再启动时可能读不到或冲突。

若已误用 sudo 启动过，清理后重来：
```bash
sudo ./stop-daemon.sh 2>/dev/null || true
sudo rm -rf .cache .remotecontrol.pid remotecontrol.log
./start-daemon.sh   # 以普通用户重新启动
```

### 画面无法显示
- 检查浏览器控制台是否有 WebCodecs 错误
- 确认浏览器支持 WebCodecs（Chrome 103+）

### 鼠标坐标偏移
- 确保访问的是最新版页面（强刷 Ctrl+Shift+R）
- 已自动按屏幕比例对齐，非标准 16:9 也能正确映射

## 目录结构

```
.
├── remotecontrol-server    # 可执行文件
├── web/                     # 前端静态资源
├── config.example.toml      # 配置模板
├── start.sh                 # 前台启动脚本
├── start-daemon.sh          # 后台启动脚本
├── stop-daemon.sh           # 停止脚本
└── README.md                # 本文档
```

运行时生成：
- `config.toml` - 实际配置（需手动创建）
- `.cache/` - 自签名证书缓存
- `.remotecontrol.pid` - 后台运行 PID
- `remotecontrol.log` - 运行日志
EOF
echo "✓ README.md"
echo ""

echo "==> 7. 打包为 tar.gz..."
cd dist
tar czf "${PACKAGE_NAME}.tar.gz" "$PACKAGE_NAME"
PACKAGE_SIZE=$(du -h "${PACKAGE_NAME}.tar.gz" | cut -f1)
echo "✓ ${PACKAGE_NAME}.tar.gz ($PACKAGE_SIZE)"
cd ..
echo ""

echo "==> 8. 生成打包清单..."
tree -L 2 "$PACKAGE_DIR" > "$PACKAGE_DIR.list.txt" 2>/dev/null || find "$PACKAGE_DIR" -maxdepth 2 -print | sort > "$PACKAGE_DIR.list.txt"
echo "✓ 文件清单: dist/${PACKAGE_NAME}.list.txt"
echo ""

echo "=========================================="
echo "✓ 打包完成！"
echo ""
echo "压缩包: dist/${PACKAGE_NAME}.tar.gz ($PACKAGE_SIZE)"
echo "源目录: $PACKAGE_DIR"
echo ""
echo "分发给其他电脑使用:"
echo "  1. scp dist/${PACKAGE_NAME}.tar.gz user@remote:/path/"
echo "  2. ssh user@remote"
echo "  3. tar xzf ${PACKAGE_NAME}.tar.gz"
echo "  4. cd $PACKAGE_NAME"
echo "  5. cp config.example.toml config.toml && nano config.toml"
echo "  6. ./start-daemon.sh"
echo "=========================================="
