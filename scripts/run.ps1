# RemoteControl 一键启动脚本 (Windows PowerShell, 前台运行)

$ErrorActionPreference = "Stop"

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
$ScriptDir = $PSScriptRoot
if (Test-Path "$ScriptDir\remotecontrol-server.exe") {
    # 打包模式
    $WorkDir = $ScriptDir
    $RunCmd = ".\remotecontrol-server.exe"
} else {
    # 开发模式
    $WorkDir = Split-Path -Parent $ScriptDir
    $RunCmd = "cargo run --release --"
}

Set-Location $WorkDir

# 打包模式下检查配置文件
if ((Test-Path "$ScriptDir\remotecontrol-server.exe") -and (-not (Test-Path "config.toml"))) {
    Write-Error "错误: config.toml 不存在，请先执行:`n  Copy-Item config.example.toml config.toml"
    exit 1
}

$ProcessName = "remotecontrol-server"
$PidFile = "$WorkDir\.remotecontrol.pid"

Write-Host "==> 检查已有进程..."
$ExistingProcess = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue
if ($ExistingProcess) {
    Write-Host "    发现已运行的进程: PID $($ExistingProcess.Id)，正在停止..."
    Stop-Process -Name $ProcessName -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Write-Host "    已停止旧进程"
}

Remove-Item $PidFile -ErrorAction SilentlyContinue

# 开发模式下编译
if (-not (Test-Path "$ScriptDir\remotecontrol-server.exe")) {
    Write-Host "==> 编译项目..."
    cargo build --release 2>&1 | Select-Object -Last 5
}

# 获取本机 IP
$LocalIP = (Get-NetIPAddress -AddressFamily IPv4 -InterfaceAlias "Wi-Fi","Ethernet","以太网" -ErrorAction SilentlyContinue | Select-Object -First 1).IPAddress
if (-not $LocalIP) {
    $LocalIP = "127.0.0.1"
}

Write-Host "==> 启动 RemoteControl 服务..."
Write-Host "    工作目录: $WorkDir"
Write-Host "    访问地址: https://$LocalIP:8443"
Write-Host "    按 Ctrl+C 停止服务"
Write-Host ""

# 前台运行
if ($RunCmd -like "cargo*") {
    Invoke-Expression $RunCmd
} else {
    & "$WorkDir\remotecontrol-server.exe" $args
}