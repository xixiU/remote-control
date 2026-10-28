# RemoteControl 后台服务停止脚本 (Windows PowerShell)

$ErrorActionPreference = "Stop"

# 目录检测：主程序在同目录 → 打包模式；否则 → 开发模式
$ScriptDir = $PSScriptRoot
if (Test-Path "$ScriptDir\remotecontrol-server.exe") {
    $WorkDir = $ScriptDir
} else {
    $WorkDir = Split-Path -Parent $ScriptDir
}

Set-Location $WorkDir

$ProcessName = "remotecontrol-server"
$PidFile = "$WorkDir\.remotecontrol.pid"

Write-Host "==> 停止 RemoteControl 服务..."

# 从 PID 文件停止
if (Test-Path $PidFile) {
    $Pid = Get-Content $PidFile
    $Process = Get-Process -Id $Pid -ErrorAction SilentlyContinue
    if ($Process) {
        Write-Host "    停止进程 $Pid..."
        Stop-Process -Id $Pid -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
    Remove-Item $PidFile -ErrorAction SilentlyContinue
}

# 清理残留进程
$RemainingProcess = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue
if ($RemainingProcess) {
    Write-Host "    清理残留进程: $($RemainingProcess.Id)"
    Stop-Process -Name $ProcessName -Force -ErrorAction SilentlyContinue
}

Write-Host "✓ 服务已停止"