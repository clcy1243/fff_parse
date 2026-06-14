#requires -Version 5
$ErrorActionPreference = "Stop"

# 脚本位于 scripts/，解析 repo 根目录
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# 版本号来自 Cargo.toml 单一真源
$m = Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $m) { throw "无法从 Cargo.toml 解析 version" }
$version = $m.Matches.Groups[1].Value
Write-Host "Building FFF Viewer v$version ..."

# 1. 编译 release 二进制（MSVC 工具链）
cargo build --release --bin fff_viewer
if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }

# 2. 定位 ISCC.exe（Inno Setup 6）
$iscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) { $iscc = Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe" }
if (-not (Test-Path $iscc)) {
    throw "未找到 Inno Setup 6 (ISCC.exe)。请安装：https://jrsoftware.org/isdl.php"
}

# 3. 编译安装包（版本号注入 .iss）
& $iscc "/DMyAppVersion=$version" "installer\windows\fff-viewer.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC 编译失败" }

Write-Host "Installer: dist\FFF Viewer-$version-setup.exe"
