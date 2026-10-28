# RemoteControl 构建脚本 (Windows PowerShell)
# 用法:
#   .\scripts\build.ps1              # release 构建
#   .\scripts\build.ps1 -Debug       # debug 构建
#   .\scripts\build.ps1 -Test        # 构建 + 跑测试
#   .\scripts\build.ps1 -Clean       # 先清理再构建

param(
    [switch]$Debug,
    [switch]$Release,
    [switch]$Test,
    [switch]$Clean
)

$ErrorActionPreference = "Stop"

# 项目根目录
$ProjectDir = Split-Path -Parent $PSScriptRoot
Set-Location $ProjectDir

# 解析参数
$Profile = if ($Debug) { "debug" } else { "release" }

# 检查 cargo
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "错误: 未找到 cargo，请先安装 Rust (https://rustup.rs)"
    exit 1
}

Write-Host "==> Rust 版本: $(rustc --version)"

if ($Clean) {
    Write-Host "==> 清理 target 目录..."
    cargo clean
}

# 构建
if ($Profile -eq "release") {
    Write-Host "==> Release 构建..."
    cargo build --release
    $BinPath = "$ProjectDir\target\release\remotecontrol-server.exe"
} else {
    Write-Host "==> Debug 构建..."
    cargo build
    $BinPath = "$ProjectDir\target\debug\remotecontrol-server.exe"
}

# 测试
if ($Test) {
    Write-Host "==> 运行单元测试..."
    cargo test
}

Write-Host ""
Write-Host "==> 构建完成"
Write-Host "    二进制: $BinPath"
if (Test-Path $BinPath) {
    $Size = (Get-Item $BinPath).Length / 1MB
    Write-Host "    大小:   $($Size.ToString('0.00')) MB"
}
Write-Host "    启动:   .\scripts\run.ps1"