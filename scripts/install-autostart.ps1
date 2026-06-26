# RemoteControl 开机自启安装脚本 (Windows PowerShell)
#
# 采用注册表方式：写入 HKCU\Run，用户登录后自动启动
# 无需管理员权限，绑定当前用户会话
#
# 用法:
#   .\install-autostart.ps1          # 安装
#   .\install-autostart.ps1 -Remove  # 卸载

param(
    [switch]$Remove
)

$ErrorActionPreference = "Stop"

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
$ScriptDir = $PSScriptRoot
if (Test-Path "$ScriptDir\remotecontrol-server.exe") {
    # 打包模式：脚本与主程序同目录
    $PkgDir = $ScriptDir
    $StartScript = "$PkgDir\start-daemon.ps1"
    $ExePath = "$PkgDir\remotecontrol-server.exe"
} else {
    # 开发模式：脚本在 scripts/ 下
    $PkgDir = Split-Path -Parent $ScriptDir
    $StartScript = "$PkgDir\scripts\start-daemon.ps1"
    $ExePath = "$PkgDir\target\release\remotecontrol-server.exe"
}

$RegPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$RegName = "RemoteControl"

if ($Remove) {
    Write-Host "==> 卸载开机自启..."
    $Existing = Get-ItemProperty -Path $RegPath -Name $RegName -ErrorAction SilentlyContinue
    if ($Existing) {
        Remove-ItemProperty -Path $RegPath -Name $RegName -ErrorAction SilentlyContinue
        Write-Host "✓ 已删除自启项"
    } else {
        Write-Host "  未找到自启项"
    }
    Write-Host "  （已运行的服务不受影响，停止请执行 .\stop-daemon.ps1）"
    exit 0
}

Write-Host "==> 安装开机自启（用户级，无需管理员）"

# 检查必要文件
if (-not (Test-Path $ExePath)) {
    Write-Error "✗ 未找到可执行文件: $ExePath`n  请先运行 .\build.ps1 编译项目"
    exit 1
}

if (-not (Test-Path "$PkgDir\config.toml")) {
    Write-Host "⚠ config.toml 不存在，建议先: Copy-Item config.example.toml config.toml 并改密码"
}

# 创建启动命令
# 使用 PowerShell 启动，延迟 5 秒后执行
$StartCommand = "powershell.exe -WindowStyle Hidden -Command \"Start-Sleep -Seconds 5; Set-Location '$PkgDir'; & '$ExePath'\""

# 写入注册表
Set-ItemProperty -Path $RegPath -Name $RegName -Value $StartCommand -Type String

Write-Host "✓ 已写入自启项: $RegPath\$RegName"
Write-Host "  登录 Windows 后约 5 秒自动后台启动"
Write-Host "  日志: $PkgDir\remotecontrol.log"
Write-Host "  立即测试: $StartScript"
Write-Host "  卸载自启: .\install-autostart.ps1 -Remove"

# 验证注册表项
$Verify = Get-ItemProperty -Path $RegPath -Name $RegName -ErrorAction SilentlyContinue
if ($Verify) {
    Write-Host ""
    Write-Host "==> 注册表项内容:"
    Write-Host "  $($Verify.$RegName)"
}