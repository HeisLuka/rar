Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$pidPath = Join-Path $repoRoot ".chaptera-local\chaptera.pid"

if (-not (Test-Path -LiteralPath $pidPath)) {
    Write-Host "Chaptera Local is not running."
    exit 0
}

$raw = (Get-Content -LiteralPath $pidPath -Raw).Trim()
if ($raw -notmatch '^\d+$') {
    Remove-Item -LiteralPath $pidPath -Force
    throw "invalid Chaptera Local pid file"
}

$process = Get-Process -Id ([int]$raw) -ErrorAction SilentlyContinue
if ($null -eq $process) {
    Remove-Item -LiteralPath $pidPath -Force
    Write-Host "Chaptera Local process is already gone."
    exit 0
}

Write-Host "Stopping Chaptera Local PID $raw..."
Stop-Process -Id $process.Id
$process.WaitForExit()
Remove-Item -LiteralPath $pidPath -Force -ErrorAction SilentlyContinue
Write-Host "Stopped."
