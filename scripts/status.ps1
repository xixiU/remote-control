# RemoteControl 后台服务状态查询脚本 (Windows PowerShell)

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
$LogFile = "$WorkDir\remotecontrol.log"

Write-Host "==> RemoteControl 服务状态"
Write-Host ""

# 检查进程状态
$Process = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue

if (Test-Path $PidFile) {
    $Pid = Get-Content $PidFile
    $PidProcess = Get-Process -Id $Pid -ErrorAction SilentlyContinue

    if ($PidProcess) {
        Write-Host "✓ 服务运行中"
        Write-Host "  PID: $Pid"
        Write-Host "  启动时间: $($PidProcess.StartTime)"
        $MemoryMB = $PidProcess.WorkingSet64 / 1MB
        Write-Host "  内存占用: $($MemoryMB.ToString('0.1')) MB"
        Write-Host "  CPU: $($PidProcess.CPU) 秒（累计）"
    } else {
        Write-Host "✗ 服务未运行（PID 文件存在但进程已退出）"
        Write-Host "  旧 PID: $Pid"
    }
} elseif ($Process) {
    Write-Host "⚠ 服务运行中（但无 PID 文件）"
    Write-Host "  PID: $($Process.Id)"
    Write-Host "  可能是手动启动的，运行 .\stop-daemon.ps1 可停止"
} else {
    Write-Host "✗ 服务未运行"
}

Write-Host ""
Write-Host "==> 日志文件"
if (Test-Path $LogFile) {
    $LogSize = (Get-Item $LogFile).Length / 1KB
    Write-Host "  路径: $LogFile"
    Write-Host "  大小: $($LogSize.ToString('0.0')) KB"
    Write-Host "  最后 5 行:"
    Write-Host "  ---"
    Get-Content $LogFile -Tail 5 | ForEach-Object { Write-Host "  $_" }
} else {
    Write-Host "  日志文件不存在"
}

Write-Host ""
Write-Host "==> 端口监听"
$PortListening = Get-NetTCPConnection -LocalPort 8443 -State Listen -ErrorAction SilentlyContinue
if ($PortListening) {
    Write-Host "  ✓ 端口 8443 正在监听"
    $PortListening | ForEach-Object {
        Write-Host "    本地地址: $($_.LocalAddress):$($_.LocalPort)"
        $OwnerProcess = Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue
        if ($OwnerProcess) {
            Write-Host "    进程: $($OwnerProcess.ProcessName) (PID: $($OwnerProcess.Id))"
        }
    }
} else {
    Write-Host "  未检测到端口 8443 监听"
}