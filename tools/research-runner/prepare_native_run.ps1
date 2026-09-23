param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

$ErrorActionPreference = "Stop"

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Resolve-PublisherExecutable {
    $candidates = @()
    foreach ($view in @("Registry64", "Registry32")) {
        try {
            $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
                [Microsoft.Win32.RegistryHive]::ClassesRoot,
                [Microsoft.Win32.RegistryView]::$view
            )
            $key = $base.OpenSubKey("Publisher.Application\CLSID")
            if ($null -ne $key) {
                $clsid = [string]$key.GetValue("")
                $key.Dispose()
                if (-not [string]::IsNullOrWhiteSpace($clsid)) {
                    $server = $base.OpenSubKey("CLSID\$clsid\LocalServer32")
                    if ($null -ne $server) {
                        $raw = [string]$server.GetValue("")
                        $server.Dispose()
                        if (-not [string]::IsNullOrWhiteSpace($raw)) {
                            $expanded = [Environment]::ExpandEnvironmentVariables($raw).Trim()
                            if ($expanded.StartsWith('"')) {
                                $end = $expanded.IndexOf('"', 1)
                                if ($end -gt 1) { $expanded = $expanded.Substring(1, $end - 1) }
                            } else {
                                $match = [regex]::Match($expanded, '^(.*?\.exe)(?:\s|$)', 'IgnoreCase')
                                if ($match.Success) { $expanded = $match.Groups[1].Value }
                            }
                            if (Test-Path -LiteralPath $expanded -PathType Leaf) { $candidates += $expanded }
                        }
                    }
                }
            }
            $base.Dispose()
        } catch {
            Write-Verbose "Publisher registry probe failed for $view"
        }
    }

    $fallbacks = @(
        "$env:ProgramFiles\Microsoft Office\root\Office16\MSPUB.EXE",
        "${env:ProgramFiles(x86)}\Microsoft Office\root\Office16\MSPUB.EXE",
        "$env:ProgramFiles\Microsoft Office\Office16\MSPUB.EXE",
        "${env:ProgramFiles(x86)}\Microsoft Office\Office16\MSPUB.EXE"
    )
    foreach ($path in $fallbacks) {
        if ($path -and (Test-Path -LiteralPath $path -PathType Leaf)) { $candidates += $path }
    }

    $unique = @($candidates | Select-Object -Unique)
    if ($unique.Count -eq 0) { return $null }
    if ($unique.Count -gt 1) {
        throw "Multiple Publisher executables found; runner is not version-pinned: $($unique -join '; ')"
    }
    return $unique[0]
}

$packet = Get-Content -LiteralPath $PacketPath -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
foreach ($name in @("logs", "analysis", "private")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $OutputRoot $name) | Out-Null
}

$packetHash = Get-Sha256 $PacketPath
if ($env:PUB_RESEARCH_PACKET_SHA256 -and $packetHash -ne $env:PUB_RESEARCH_PACKET_SHA256.ToLowerInvariant()) {
    throw "Packet SHA-256 changed after hosted validation."
}

$resetInfo = [ordered]@{ required = $false }
if ($null -ne $packet.reset -and [bool]$packet.reset.required) {
    $providerPath = [Environment]::ExpandEnvironmentVariables([string]$env:PUB_RESEARCH_RESET_PROVIDER)
    if ([string]::IsNullOrWhiteSpace($providerPath)) {
        throw "PUB_RESEARCH_RESET_PROVIDER is required when packet.reset.required=true."
    }
    if (-not (Test-Path -LiteralPath $providerPath -PathType Leaf)) {
        throw "Configured reset provider does not exist: $providerPath"
    }
    if ([IO.Path]::GetExtension($providerPath) -ne ".ps1") {
        throw "PUB_RESEARCH_RESET_PROVIDER must point to a PowerShell .ps1 provider adapter."
    }

    $receiptPath = Join-Path $OutputRoot "analysis\reset-receipt.json"
    & pwsh -NoProfile -File $providerPath `
        -BaselineId ([string]$packet.reset.baseline_id) `
        -SnapshotId ([string]$packet.reset.snapshot_id) `
        -ExperimentId ([string]$packet.id) `
        -PacketSha256 $packetHash `
        -ReceiptPath $receiptPath
    if ($LASTEXITCODE -ne 0) {
        throw "Reset provider failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
        throw "Reset provider did not emit the required receipt."
    }

    python tools/research-runner/verify_reset_receipt.py `
        --receipt $receiptPath `
        --expected-baseline ([string]$packet.reset.baseline_id) `
        --expected-snapshot ([string]$packet.reset.snapshot_id) `
        --expected-experiment ([string]$packet.id) `
        --expected-packet-sha256 $packetHash
    if ($LASTEXITCODE -ne 0) {
        throw "Reset receipt verification failed."
    }

    $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    $resetInfo = [ordered]@{
        required = $true
        provider_id = [string]$receipt.provider_id
        provider_version = [string]$receipt.provider_version
        baseline_id = [string]$receipt.baseline_id
        snapshot_id = [string]$receipt.snapshot_id
        environment_fingerprint_sha256 = [string]$receipt.environment_fingerprint_sha256
        restore_verified = [bool]$receipt.restore_verified
        receipt_sha256 = Get-Sha256 $receiptPath
    }
}

$publisherInfo = $null
if ([bool]$packet.requires_publisher) {
    $publisherExe = Resolve-PublisherExecutable
    if (-not $publisherExe) { throw "Publisher.Application / MSPUB.EXE not found." }

    $version = [Diagnostics.FileVersionInfo]::GetVersionInfo($publisherExe)
    $publisherHash = Get-Sha256 $publisherExe
    if ($packet.publisher.version_prefix -and -not $version.FileVersion.StartsWith([string]$packet.publisher.version_prefix)) {
        throw "Publisher version mismatch: expected prefix $($packet.publisher.version_prefix), got $($version.FileVersion)"
    }
    if ($packet.publisher.exe_sha256 -and $publisherHash -ne ([string]$packet.publisher.exe_sha256).ToLowerInvariant()) {
        throw "Publisher executable SHA-256 mismatch."
    }

    $publisherInfo = [ordered]@{
        file_version = $version.FileVersion
        product_version = $version.ProductVersion
        sha256 = $publisherHash
        executable_name = [IO.Path]::GetFileName($publisherExe)
    }
    $env:PUB_RESEARCH_PUBLISHER_EXE = $publisherExe
}

$fixtureInfo = $null
if ($null -ne $packet.fixture) {
    if ($packet.fixture.source -eq "repo") {
        $fixtureRoot = (Get-Location).Path
    } else {
        $fixtureRoot = $env:PUB_RESEARCH_FIXTURE_ROOT
        if ([string]::IsNullOrWhiteSpace($fixtureRoot)) {
            throw "PUB_RESEARCH_FIXTURE_ROOT is required for runner-root fixtures."
        }
    }
    $fixturePath = Join-Path $fixtureRoot ([string]$packet.fixture.relative_path)
    if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) {
        throw "Fixture not found: $($packet.fixture.relative_path)"
    }
    $fixtureHash = Get-Sha256 $fixturePath
    if ($packet.fixture.sha256 -and $fixtureHash -ne ([string]$packet.fixture.sha256).ToLowerInvariant()) {
        throw "Fixture SHA-256 mismatch."
    }
    $fixtureInfo = [ordered]@{
        source = [string]$packet.fixture.source
        relative_path = [string]$packet.fixture.relative_path
        sha256 = $fixtureHash
        size = (Get-Item -LiteralPath $fixturePath).Length
    }
    $env:PUB_RESEARCH_FIXTURE = $fixturePath
}

$os = Get-CimInstance Win32_OperatingSystem
$environment = [ordered]@{
    schema = "pub-research-environment.v1"
    experiment_id = [string]$packet.id
    publisher_environment = [string]$packet.publisher_environment
    packet_sha256 = $packetHash
    git_sha = $env:GITHUB_SHA
    runner_name = $env:RUNNER_NAME
    runner_arch = $env:RUNNER_ARCH
    os = [ordered]@{
        caption = $os.Caption
        version = $os.Version
        build_number = $os.BuildNumber
    }
    reset = $resetInfo
    publisher = $publisherInfo
    fixture = $fixtureInfo
}
$environment | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputRoot "environment.json") -Encoding UTF8
Write-Host "Native research environment verified for $($packet.id)."
