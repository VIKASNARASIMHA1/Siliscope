# Windows launcher.  Usage:  .\rvsim.ps1 <build | run | selftest | test | bench | clean>
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$CommandArgs)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

foreach ($tool in @("cargo", "clang")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Write-Host "Missing tool: $tool" -ForegroundColor Red
        Write-Host "  Rust : winget install Rustlang.Rustup     (then: rustup default stable-msvc)"
        Write-Host "  LLVM : winget install LLVM.LLVM           (provides clang + lld)"
        Write-Host "Close and reopen PowerShell after installing, then run this script again."
        exit 1
    }
}
# If this folder was extracted over an older version, cargo can wrongly think its old build
# outputs are up to date (zip timestamps are older). Wipe them once whenever VERSION changes.
$ver = (Get-Content (Join-Path $PSScriptRoot "VERSION") -Raw).Trim()
$stampFile = Join-Path $PSScriptRoot "target\.rvsim_version"
$old = ""
if (Test-Path $stampFile) { $old = (Get-Content $stampFile -Raw).Trim() }
if ($old -ne $ver) {
    Write-Host "rvsim $ver : removing stale build files from an older version (one-time)..."
    foreach ($d in @("target", "build")) {
        $p = Join-Path $PSScriptRoot $d
        if (Test-Path $p) { Remove-Item -Recurse -Force $p }
    }
    New-Item -ItemType Directory -Force (Join-Path $PSScriptRoot "target") | Out-Null
    Set-Content -Path $stampFile -Value $ver
}
if ($CommandArgs.Count -eq 0) { $CommandArgs = @("help") }
cargo run --release -q -p xtask -- @CommandArgs
exit $LASTEXITCODE
