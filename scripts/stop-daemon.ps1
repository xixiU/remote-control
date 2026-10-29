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


# ============================================================================
# 辅助函数：优雅停止进程（先 taskkill 不带 /F，超时后 /F 兜底）
# ============================================================================
# 为什么不用 Stop-Process：
#   Stop-Process 即便去掉 -Force，对无窗口进程仍然调用 TerminateProcess，
#   不会发送 CTRL_CLOSE_EVENT，导致服务端的退出清理（释放残留按键）不生效。
#   taskkill 不带 /F 会尝试发 CTRL_CLOSE_EVENT，让进程有机会执行清理后优雅退出。
#
# 两阶段策略：
#   1) taskkill /PID xxx（不带 /F）：发 CTRL_CLOSE_EVENT，进程有约 5s 窗口清理
#   2) 轮询等待最多 3 秒：正常情况清理只需毫秒级，进程会快速退出
#   3) 超时仍未退出：taskkill /F 强杀兜底，打印提示说明走了强杀路径
#
# 注意 PowerShell 5.1 的陷阱：
#   原生 exe 的 stderr 若用 2>&1 重定向会被包成 ErrorRecord，在 EAP=Stop 下
#   变成终止错误（即使退出码 0）。这里通过 ProcessStartInfo 的 RedirectStandardError
#   把 stderr 吸收掉，只用进程退出码判断结果。
function Stop-ProcessGracefully {
    param([int]$ProcessId)
    
    # 第一阶段：优雅停止（taskkill 不带 /F）
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = "taskkill.exe"
    $psi.Arguments = "/PID $ProcessId"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $psi.CreateNoWindow = $true
    
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    $null = $proc.Start()
    $proc.WaitForExit()
    $exitCode = $proc.ExitCode
    
    if ($exitCode -eq 0) {
        # 轮询等待进程退出，最多 3 秒
        $maxWait = 15
        $waited = 0
        while ($waited -lt $maxWait) {
            $running = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
            if (-not $running) {
                return
            }
            Start-Sleep -Milliseconds 200
            $waited = $waited + 1
        }
        
        # 超时仍未退出，走强杀路径
        Write-Host "        进程 $ProcessId 超时未退出，强制结束"
        $psi2 = New-Object System.Diagnostics.ProcessStartInfo
        $psi2.FileName = "taskkill.exe"
        $psi2.Arguments = "/F /PID $ProcessId"
        $psi2.UseShellExecute = $false
        $psi2.RedirectStandardError = $true
        $psi2.RedirectStandardOutput = $true
        $psi2.CreateNoWindow = $true
        
        $proc2 = New-Object System.Diagnostics.Process
        $proc2.StartInfo = $psi2
        $null = $proc2.Start()
        $proc2.WaitForExit()
        
        if ($proc2.ExitCode -ne 0) {
            Write-Host "        警告: 强制结束失败，进程可能已退出"
        }
    } else {
        # taskkill 不带 /F 失败，直接尝试 /F 强杀
        $psi3 = New-Object System.Diagnostics.ProcessStartInfo
        $psi3.FileName = "taskkill.exe"
        $psi3.Arguments = "/F /PID $ProcessId"
        $psi3.UseShellExecute = $false
        $psi3.RedirectStandardError = $true
        $psi3.RedirectStandardOutput = $true
        $psi3.CreateNoWindow = $true
        
        $proc3 = New-Object System.Diagnostics.Process
        $proc3.StartInfo = $psi3
        $null = $proc3.Start()
        $proc3.WaitForExit()
        
        if ($proc3.ExitCode -ne 0) {
            Write-Host "        进程 $ProcessId 无法停止，可能已退出"
        }
    }
}

Write-Host "==> 停止 RemoteControl 服务..."

# 从 PID 文件停止（避免用 $Pid 变量名，它是只读的自动变量会在 EAP=Stop 下抛错）
if (Test-Path $PidFile) {
    $StoredPid = Get-Content $PidFile
    $Process = Get-Process -Id $StoredPid -ErrorAction SilentlyContinue
    if ($Process) {
        Write-Host "    停止进程 $StoredPid..."
        Stop-ProcessGracefully $StoredPid
    }
    Remove-Item $PidFile -ErrorAction SilentlyContinue
}

# 清理残留进程
$RemainingProcess = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue
if ($RemainingProcess) {
    Write-Host "    清理残留进程: $($RemainingProcess.Id)"
    foreach ($p in $RemainingProcess) {
        Stop-ProcessGracefully $p.Id
    }
}

Write-Host "✓ 服务已停止"
