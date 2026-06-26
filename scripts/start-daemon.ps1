# RemoteControl 后台启动脚本 (Windows PowerShell)

$ErrorActionPreference = "Stop"

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
$ScriptDir = $PSScriptRoot
if (Test-Path "$ScriptDir\remotecontrol-server.exe") {
    # 打包模式
    $WorkDir = $ScriptDir
    $ExePath = "$WorkDir\remotecontrol-server.exe"
} else {
    # 开发模式
    $WorkDir = Split-Path -Parent $ScriptDir
    # 检查是否需要编译
    $ExePath = "$WorkDir\target\release\remotecontrol-server.exe"
    if (-not (Test-Path $ExePath)) {
        Write-Host "==> 编译项目..."
        cargo build --release 2>&1 | Select-Object -Last 5
    }
}

Set-Location $WorkDir

# 打包模式下检查配置文件
if ((Test-Path "$ScriptDir\remotecontrol-server.exe") -and (-not (Test-Path "config.toml"))) {
    Write-Error "错误: config.toml 不存在，请先执行:`n  Copy-Item config.example.toml config.toml"
    exit 1
}

$ProcessName = "remotecontrol-server"
$PidFile = "$WorkDir\.remotecontrol.pid"
$LogFile = "$WorkDir\remotecontrol.log"

Write-Host "==> 检查已有进程..."
$ExistingProcess = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue
if ($ExistingProcess) {
    Write-Host "    发现已运行的进程: PID $($ExistingProcess.Id)，正在停止..."
    Stop-Process -Name $ProcessName -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Write-Host "    已停止旧进程"
}

Remove-Item $PidFile -ErrorAction SilentlyContinue

# 获取本机 IP
$LocalIP = (Get-NetIPAddress -AddressFamily IPv4 -InterfaceAlias "Wi-Fi","Ethernet","以太网" -ErrorAction SilentlyContinue | Select-Object -First 1).IPAddress
if (-not $LocalIP) {
    $LocalIP = "127.0.0.1"
}

Write-Host "==> 启动服务（后台模式）..."
Write-Host "    工作目录: $WorkDir"
Write-Host "    日志: $LogFile"
Write-Host "    访问: https://$LocalIP:8443"
Write-Host ""

# 后台启动
$Process = Start-Process -FilePath $ExePath -WorkingDirectory $WorkDir -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err" -PassThru -WindowStyle Hidden

# 保存 PID
$Process.Id | Out-File $PidFile

Start-Sleep -Seconds 2

# 检查进程是否存活
$RunningProcess = Get-Process -Id $Process.Id -ErrorAction SilentlyContinue
if ($RunningProcess) {
    Write-Host "✓ 服务已启动，PID: $($Process.Id)"
    Write-Host "  查看日志: Get-Content $LogFile -Tail 20"
    Write-Host "  停止服务: .\stop-daemon.ps1"
} else {
    Write-Host "✗ 启动失败，请查看日志: $LogFile"
    Remove-Item $PidFile -ErrorAction SilentlyContinue
    if (Test-Path $LogFile) {
        Write-Host "日志内容:"
        Get-Content $LogFile -Tail 10
    }
    exit 1
}