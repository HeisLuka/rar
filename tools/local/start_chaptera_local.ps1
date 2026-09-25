param(
    [string]$Config,
    [string]$Url = "http://127.0.0.1:8080",
    [switch]$SkipBuild,
    [switch]$Migrate,
    [switch]$NoBrowser
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$stateRoot = Join-Path $repoRoot ".chaptera-local"
$logRoot = Join-Path $stateRoot "logs"
$pidPath = Join-Path $stateRoot "chaptera.pid"
New-Item -ItemType Directory -Force -Path $logRoot | Out-Null

if (Test-Path -LiteralPath $pidPath) {
    $existingPid = (Get-Content -LiteralPath $pidPath -Raw).Trim()
    if ($existingPid -match '^\d+$') {
        $existing = Get-Process -Id ([int]$existingPid) -ErrorAction SilentlyContinue
        if ($null -ne $existing) {
            Write-Host "Chaptera Local is already running as PID $existingPid"
            if (-not $NoBrowser) {
                Start-Process ($Url.TrimEnd("/") + "/__chaptera")
            }
            exit 0
        }
    }
    Remove-Item -LiteralPath $pidPath -Force -ErrorAction SilentlyContinue
}

Push-Location $repoRoot
try {
    if (-not $SkipBuild) {
        Write-Host "Building Chaptera server..."
        cargo build -p chaptera-server --bin chaptera
        if ($LASTEXITCODE -ne 0) {
            throw "chaptera build failed"
        }
    }

    $exe = Join-Path $repoRoot "target\debug\chaptera.exe"
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
        throw "chaptera.exe not found: $exe"
    }

    $resolvedConfig = $null
    if ($Config) {
        $resolvedConfig = (Resolve-Path -LiteralPath $Config).Path
        if ($Migrate) {
            Write-Host "Applying local schema migrations..."
            & $exe --config $resolvedConfig migrate up
            if ($LASTEXITCODE -ne 0) {
                throw "chaptera migrate up failed"
            }
        }
    }

    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $stdoutPath = Join-Path $logRoot "chaptera-$stamp.stdout.log"
    $stderrPath = Join-Path $logRoot "chaptera-$stamp.jsonl"

    $arguments = @()
    if ($resolvedConfig) {
        $arguments += @("--config", $resolvedConfig)
    }
    $arguments += "local"

    Write-Host "Starting Chaptera Local..."
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -WorkingDirectory $repoRoot -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru

    Set-Content -LiteralPath $pidPath -Value $process.Id -Encoding ascii

    $liveUrl = $Url.TrimEnd("/") + "/live"
    $consoleUrl = $Url.TrimEnd("/") + "/__chaptera"
    $deadline = (Get-Date).AddSeconds(45)
    $live = $false
    while ((Get-Date) -lt $deadline) {
        if ($process.HasExited) {
            Write-Host "Chaptera exited before becoming live." -ForegroundColor Red
            if (Test-Path -LiteralPath $stderrPath) {
                Get-Content -LiteralPath $stderrPath -Tail 40
            }
            throw "chaptera local exited with code $($process.ExitCode)"
        }
        try {
            $response = Invoke-WebRequest -Uri $liveUrl -UseBasicParsing -TimeoutSec 2
            if ($response.StatusCode -eq 200) {
                $live = $true
                break
            }
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }

    if (-not $live) {
        Write-Host "Chaptera did not become live within 45 seconds." -ForegroundColor Red
        Write-Host "Log: $stderrPath"
        throw "chaptera local startup timeout"
    }

    Write-Host ""
    Write-Host "Chaptera Local is running." -ForegroundColor Green
    Write-Host "Console: $consoleUrl"
    Write-Host "JSON log: $stderrPath"
    Write-Host "PID: $($process.Id)"
    Write-Host "Stop: pwsh -File tools/local/stop_chaptera_local.ps1"
    if ($resolvedConfig) {
        Write-Host "Config: $resolvedConfig"
    } else {
        Write-Host "Mode: unconfigured development shell (console will show missing producers)" -ForegroundColor Yellow
    }

    if (-not $NoBrowser) {
        Start-Process $consoleUrl
    }
}
finally {
    Pop-Location
}
