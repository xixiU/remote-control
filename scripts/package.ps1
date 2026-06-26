# RemoteControl 打包脚本 (Windows PowerShell)
# 生成可分发的二进制包，包含可执行文件、静态资源、配置示例、运行脚本

$ErrorActionPreference = "Stop"

# 项目根目录
$ProjectDir = Split-Path -Parent $PSScriptRoot
Set-Location $ProjectDir

# 打包输出目录
$Arch = if ($env:PROCESSOR_ARCHITECTURE -eq "AMD64") { "x86_64" } else { "x86" }
$DateStr = Get-Date -Format "yyyyMMdd-HHmmss"
$PackageName = "remotecontrol-$Arch-win-$DateStr"
$PackageDir = "dist\$PackageName"

Write-Host "==> RemoteControl 打包脚本 (Windows)"
Write-Host "    目标: $PackageDir"
Write-Host ""

# 清理旧的打包目录
if (Test-Path $PackageDir) {
    Remove-Item $PackageDir -Recurse -Force
}
New-Item -ItemType Directory -Path $PackageDir -Force | Out-Null

Write-Host "==> 1. 编译 release 二进制..."
cargo build --release 2>&1 | Select-Object -Last 5
Write-Host "✓ 编译完成"
Write-Host ""

Write-Host "==> 2. 复制二进制文件..."
$BinPath = "$ProjectDir\target\release\remotecontrol-server.exe"
Copy-Item $BinPath $PackageDir
$BinSize = (Get-Item "$PackageDir\remotecontrol-server.exe").Length / 1MB
Write-Host "✓ 二进制: $($BinSize.ToString('0.00')) MB"
Write-Host ""

Write-Host "==> 3. 复制静态资源..."
Copy-Item -Path "$ProjectDir\web" -Destination $PackageDir -Recurse
Write-Host "✓ web\ 目录已复制"
Write-Host ""

Write-Host "==> 4. 生成配置文件示例..."
$ConfigExample = @"
# RemoteControl 配置文件
# 使用: Copy-Item config.example.toml config.toml 并按需修改

bind = "0.0.0.0:8443"
password = "请修改为复杂密码"

[video]
fps = 15
# 码率 (Mbps)。设为 -1 表示按分辨率与帧率自动估算（推荐）；
# 也可手动指定，例如 8 表示 8 Mbps。桌面文字偏糊时调高此值。
bitrate = -1
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
"@
Set-Content -Path "$PackageDir\config.example.toml" -Value $ConfigExample
Write-Host "✓ config.example.toml"
Write-Host ""

Write-Host "==> 5. 复制运行脚本..."
# 复制 PowerShell 脚本
Copy-Item "$ProjectDir\scripts\run.ps1" "$PackageDir\start.ps1"
Copy-Item "$ProjectDir\scripts\start-daemon.ps1" "$PackageDir\start-daemon.ps1"
Copy-Item "$ProjectDir\scripts\stop-daemon.ps1" "$PackageDir\stop-daemon.ps1"
Copy-Item "$ProjectDir\scripts\status.ps1" "$PackageDir\status.ps1"
Copy-Item "$ProjectDir\scripts\install-autostart.ps1" "$PackageDir\install-autostart.ps1"

Write-Host "✓ start.ps1（前台运行，来自 scripts/run.ps1）"
Write-Host "✓ start-daemon.ps1（后台运行）"
Write-Host "✓ stop-daemon.ps1（停止服务）"
Write-Host "✓ status.ps1（查看状态）"
Write-Host "✓ install-autostart.ps1（开机自启）"
Write-Host ""

Write-Host "==> 6. 生成 README..."
$ReadmeContent = @"
# RemoteControl 分发包 (Windows)

RemoteControl 是一个基于 WebSocket + H.264 + WebCodecs 的远程桌面控制工具。

## 系统要求

- Windows 8 / 10 / 11（64 位）
- DirectX 11 支持的显卡
- 现代浏览器（支持 WebCodecs，如 Chrome/Edge 103+）

## 快速开始

### 1. 配置
```powershell
# 复制配置模板并修改密码
Copy-Item config.example.toml config.toml
notepad config.toml  # 修改 password 字段
```

### 2. 启动服务

**前台运行（调试用）：**
```powershell
.\start.ps1
```

**后台运行（推荐）：**
```powershell
.\start-daemon.ps1
# 查看状态
.\status.ps1
# 查看日志
Get-Content remotecontrol.log -Tail 20
# 停止服务
.\stop-daemon.ps1
```

**开机自启（可选，用户级，无需管理员）：**
```powershell
.\install-autostart.ps1          # 安装：登录 Windows 后自动启动
.\install-autostart.ps1 -Remove  # 卸载
```

### 3. 访问控制端

浏览器打开 ``https://<服务器IP>:8443``

⚠️ 首次访问会提示证书不受信任（自签名证书），点击「继续访问」或「高级 → 继续」。

输入密码后即可看到被控端画面并操作。

## 配置说明

- **bind**: 监听地址，默认 ``0.0.0.0:8443``（局域网可访问）
- **password**: 访问密码，**务必修改**
- **video.fps**: 视频帧率，默认 15
- **video.bitrate**: 视频码率，设为 -1 自动估算
- **tls.enabled**: 是否启用 HTTPS，默认 ``true``（推荐保持）

## TLS 证书

默认自动生成自签名证书并缓存到 ``.cache\`` 目录。若需自定义证书：
```toml
[tls]
enabled = true
cert = "your-cert.pem"
key = "your-key.pem"
```

## 故障排查

### 服务启动失败
```powershell
# 查看详细日志
Get-Content remotecontrol.log
```

### Desktop Duplication 错误
- 确保没有其他录屏软件占用屏幕捕获
- 检查显卡驱动是否正常

### 画面无法显示
- 检查浏览器控制台是否有 WebCodecs 错误
- 确认浏览器支持 WebCodecs（Chrome 103+）

## 目录结构

```
.
├── remotecontrol-server.exe  # 可执行文件
├── web\                       # 前端静态资源
├── config.example.toml        # 配置模板
├── start.ps1                  # 前台启动脚本
├── start-daemon.ps1           # 后台启动脚本
├── stop-daemon.ps1            # 停止脚本
├── status.ps1                 # 状态查询脚本
├── install-autostart.ps1      # 开机自启安装/卸载
└── README.md                  # 本文档
```

运行时生成：
- ``config.toml`` - 实际配置（需手动创建）
- ``.cache\`` - 自签名证书缓存
- ``.remotecontrol.pid`` - 后台运行 PID
- ``remotecontrol.log`` - 运行日志
"@
Set-Content -Path "$PackageDir\README.md" -Value $ReadmeContent
Write-Host "✓ README.md"
Write-Host ""

Write-Host "==> 7. 打包为 ZIP..."
$ZipPath = "dist\$PackageName.zip"
Compress-Archive -Path $PackageDir -DestinationPath $ZipPath -Force
$ZipSize = (Get-Item $ZipPath).Length / 1MB
Write-Host "✓ $PackageName.zip ($($ZipSize.ToString('0.00')) MB)"
Write-Host ""

Write-Host "==> 8. 生成打包清单..."
Get-ChildItem $PackageDir -Recurse | Select-Object FullName, Length | Out-File "dist\$PackageName.list.txt"
Write-Host "✓ 文件清单: dist\$PackageName.list.txt"
Write-Host ""

Write-Host "=========================================="
Write-Host "✓ 打包完成！"
Write-Host ""
Write-Host "压缩包: $ZipPath ($($ZipSize.ToString('0.00')) MB)"
Write-Host "源目录: $PackageDir"
Write-Host ""
Write-Host "分发给其他电脑使用:"
Write-Host "  1. 复制 $PackageName.zip 到目标机器"
Write-Host "  2. 解压到任意目录"
Write-Host "  3. Copy-Item config.example.toml config.toml"
Write-Host "  4. notepad config.toml（修改密码）"
Write-Host "  5. .\start-daemon.ps1"
Write-Host "=========================================="