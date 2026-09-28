param(
    [Parameter(Mandatory = $true)][string]$SetupA,
    [Parameter(Mandatory = $true)][string]$SetupB,
    [Parameter(Mandatory = $true)][string]$Fixture,
    [Parameter(Mandatory = $true)][string]$FixtureSha256,
    [int]$Cycles = 3,
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Programs\Chaptera PUB Reader"),
    [string]$OutDir = "target/lifecycle-soak",
    [string]$ForeignPubDefault = "Chaptera.Soak.PreExistingHandler"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($Cycles -lt 1) {
    throw "Cycles must be >= 1"
}

$runId = [guid]::NewGuid().ToString("N")
$startedAt = (Get-Date).ToUniversalTime()
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
New-Item -ItemType Directory -Force -Path $resolvedOut | Out-Null
$eventsPath = Join-Path $resolvedOut "events.jsonl"
$statePath = Join-Path $resolvedOut "state.json"
$summaryPath = Join-Path $resolvedOut "summary.json"
Remove-Item -Force $eventsPath,$statePath,$summaryPath -ErrorAction SilentlyContinue

$tempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$externalState = Join-Path $tempRoot "chaptera-lifecycle-soak-$runId-external-state.txt"
Set-Content -Encoding utf8 -Path $externalState -Value "chaptera-lifecycle-soak-external-state:$runId"
$externalHash = (Get-FileHash $externalState -Algorithm SHA256).Hash.ToLowerInvariant()

function Get-FileSha256([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-TreeSummary([string]$Root) {
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        return [ordered]@{
            exists = $false
            file_count = 0
            total_bytes = 0
            fingerprint_sha256 = $null
        }
    }

    $entries = @(Get-ChildItem -LiteralPath $Root -Force -Recurse -File | Sort-Object FullName)
    $lines = New-Object System.Collections.Generic.List[string]
    $bytes = [uint64]0
    foreach ($entry in $entries) {
        $relative = [IO.Path]::GetRelativePath($Root, $entry.FullName).Replace("\","/")
        $hash = (Get-FileHash -LiteralPath $entry.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $bytes += [uint64]$entry.Length
        $lines.Add(("{0}|{1}|{2}" -f $relative,$entry.Length,$hash))
    }
    $payload = [Text.Encoding]::UTF8.GetBytes(($lines -join [Environment]::NewLine))
    $sha = [Security.Cryptography.SHA256]::HashData($payload)
    $fingerprint = ([Convert]::ToHexString($sha)).ToLowerInvariant()
    return [ordered]@{
        exists = $true
        file_count = $entries.Count
        total_bytes = $bytes
        fingerprint_sha256 = $fingerprint
    }
}

function Get-RegistrySnapshot {
    $pubKey = "Registry::HKEY_CURRENT_USER\Software\Classes\.pub"
    $openWithPath = "$pubKey\OpenWithProgids"
    $commandPath = "Registry::HKEY_CURRENT_USER\Software\Classes\Chaptera.PUB.Reader\shell\open\command"
    $pubDefault = $null
    $openWith = $false
    $command = $null
    if (Test-Path $pubKey) {
        $pubDefault = (Get-Item $pubKey).GetValue("")
    }
    if (Test-Path $openWithPath) {
        $openWith = (Get-Item $openWithPath).GetValueNames() -contains "Chaptera.PUB.Reader"
    }
    if (Test-Path $commandPath) {
        $command = (Get-Item $commandPath).GetValue("")
    }
    return [ordered]@{
        pub_default = $pubDefault
        chaptera_open_with_present = $openWith
        open_command = $command
    }
}

function Get-JournalSnapshot {
    $items = [ordered]@{}
    foreach ($name in @("update-journal.json","update-journal.json.prev")) {
        $path = Join-Path $InstallRoot $name
        $exists = Test-Path -LiteralPath $path -PathType Leaf
        $items[$name] = [ordered]@{
            exists = $exists
            bytes = if ($exists) { (Get-Item -LiteralPath $path).Length } else { 0 }
            sha256 = Get-FileSha256 $path
        }
    }
    return $items
}

function Get-ProcessSnapshot {
    return @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $_.ProcessName -like "chaptera*" -or $_.ProcessName -like "*updater*"
    } | Sort-Object ProcessName,Id | ForEach-Object {
        [ordered]@{ name = $_.ProcessName; id = $_.Id }
    })
}

function Save-State([int]$Cycle, [string]$Phase, [string]$Status, [string]$ErrorMessage = $null) {
    [ordered]@{
        schema_version = "chaptera.lifecycle-soak.state.v1"
        run_id = $runId
        updated_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        cycles_total = $Cycles
        cycle = $Cycle
        phase = $Phase
        status = $Status
        error = $ErrorMessage
        install_root = $InstallRoot
        fixture_sha256 = $FixtureSha256.ToLowerInvariant()
        external_state_sha256 = $externalHash
    } | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 $statePath
}

function Write-Event(
    [int]$Cycle,
    [string]$Phase,
    [string]$Status,
    [long]$DurationMs,
    [System.Collections.IDictionary]$Extra = @{}
) {
    $current = Join-Path $InstallRoot "current"
    $event = [ordered]@{
        schema_version = "chaptera.lifecycle-soak.event.v1"
        run_id = $runId
        timestamp_utc = (Get-Date).ToUniversalTime().ToString("o")
        cycle = $Cycle
        phase = $Phase
        status = $Status
        duration_ms = $DurationMs
        installed_tree = Get-TreeSummary $current
        registry = Get-RegistrySnapshot
        journals = Get-JournalSnapshot
        processes = Get-ProcessSnapshot
        fixture_sha256 = Get-FileSha256 $Fixture
        external_state_sha256 = Get-FileSha256 $externalState
        extra = $Extra
    }
    ($event | ConvertTo-Json -Depth 10 -Compress) | Add-Content -Encoding utf8 $eventsPath
}

function Invoke-Phase(
    [int]$Cycle,
    [string]$Phase,
    [scriptblock]$Body
) {
    Save-State $Cycle $Phase "running"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        $extra = & $Body
        $sw.Stop()
        if ($null -eq $extra) { $extra = @{} }
        Write-Event $Cycle $Phase "pass" $sw.ElapsedMilliseconds $extra
        Save-State $Cycle $Phase "pass"
        return $extra
    } catch {
        $sw.Stop()
        $message = $_.Exception.Message
        Write-Event $Cycle $Phase "fail" $sw.ElapsedMilliseconds @{ error = $message }
        Save-State $Cycle $Phase "fail" $message
        throw
    }
}

function Invoke-Installer([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "installer missing: $Path"
    }
    $p = Start-Process -FilePath $Path -ArgumentList @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/SP-"
    ) -Wait -PassThru
    if ($p.ExitCode -ne 0) {
        throw "installer failed with exit code $($p.ExitCode): $Path"
    }
    return $p.ExitCode
}

function Get-ChapteraUninstallers {
    $root = "Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall"
    if (-not (Test-Path $root)) { return @() }
    return @(Get-ChildItem $root | ForEach-Object {
        $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
        if ($p.DisplayName -like "Chaptera PUB Reader *") {
            [ordered]@{
                registry_path = $_.PSPath
                display_name = $p.DisplayName
                uninstall_string = $p.UninstallString
            }
        }
    })
}

function Invoke-Uninstall {
    $entries = @(Get-ChapteraUninstallers)
    if ($entries.Count -eq 0) { return 0 }
    if ($entries.Count -ne 1) {
        throw "expected at most one Chaptera Reader uninstall entry, found $($entries.Count)"
    }
    $raw = [string]$entries[0].uninstall_string
    if ([string]::IsNullOrWhiteSpace($raw)) {
        throw "Chaptera uninstall entry has no UninstallString"
    }
    $exe = $raw.Trim().Trim('"')
    $p = Start-Process -FilePath $exe -ArgumentList @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART"
    ) -Wait -PassThru
    if ($p.ExitCode -ne 0) {
        throw "uninstaller failed with exit code $($p.ExitCode)"
    }
    return $p.ExitCode
}

function Invoke-ReaderSmoke([bool]$ExpectSuccess = $true) {
    $reader = Join-Path $InstallRoot "current\chaptera-reader.exe"
    if (-not (Test-Path -LiteralPath $reader -PathType Leaf)) {
        if ($ExpectSuccess) { throw "Reader missing: $reader" }
        return [ordered]@{ success = $false; exit_code = $null; launch_error = "missing" }
    }

    try {
        $p = Start-Process -FilePath $reader -ArgumentList @("--smoke-check",$Fixture) -Wait -PassThru
    } catch {
        if ($ExpectSuccess) { throw }
        return [ordered]@{ success = $false; exit_code = $null; launch_error = $_.Exception.Message }
    }

    $ok = $p.ExitCode -eq 0
    if ($ExpectSuccess -and -not $ok) {
        throw "Reader smoke failed with exit code $($p.ExitCode)"
    }
    if (-not $ExpectSuccess -and $ok) {
        throw "corrupted Reader unexpectedly passed smoke"
    }
    return [ordered]@{ success = $ok; exit_code = $p.ExitCode; launch_error = $null }
}

function Assert-Invariants([bool]$ExpectInstalled, [string]$ExpectedMarker = $null) {
    $fixtureHash = Get-FileSha256 $Fixture
    if ($fixtureHash -ne $FixtureSha256.ToLowerInvariant()) {
        throw "source PUB hash changed: $fixtureHash"
    }
    $stateHash = Get-FileSha256 $externalState
    if ($stateHash -ne $externalHash) {
        throw "external state changed: $stateHash"
    }
    $registry = Get-RegistrySnapshot
    if ($registry.pub_default -ne $ForeignPubDefault) {
        throw "foreign .pub default changed: $($registry.pub_default)"
    }

    $reader = Join-Path $InstallRoot "current\chaptera-reader.exe"
    if ($ExpectInstalled) {
        if (-not (Test-Path -LiteralPath $reader -PathType Leaf)) {
            throw "installed Reader missing"
        }
        if (-not $registry.chaptera_open_with_present) {
            throw "Chaptera Reader missing from Open With"
        }
        if ($registry.open_command -notlike "*current*chaptera-reader.exe*%1*") {
            throw "Open With command is not current/chaptera-reader.exe: $($registry.open_command)"
        }
        if ($ExpectedMarker) {
            $marker = Join-Path $InstallRoot "current\acceptance-version.txt"
            if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) {
                throw "expected version marker missing"
            }
            $actual = (Get-Content -Raw -LiteralPath $marker).Trim()
            if ($actual -ne $ExpectedMarker) {
                throw "expected marker $ExpectedMarker, got $actual"
            }
        }
    } else {
        if ($registry.chaptera_open_with_present) {
            throw "Chaptera Open With survived uninstall"
        }
        if (Test-Path -LiteralPath $InstallRoot) {
            $remaining = @(Get-ChildItem -LiteralPath $InstallRoot -Force -Recurse -ErrorAction SilentlyContinue)
            if ($remaining.Count -gt 0) {
                throw "install root retained content after uninstall: $($remaining.FullName -join ', ')"
            }
        }
    }
}

$fixtureActual = Get-FileSha256 $Fixture
if ($fixtureActual -ne $FixtureSha256.ToLowerInvariant()) {
    throw "fixture SHA-256 mismatch before run: $fixtureActual"
}

$pubKey = "Registry::HKEY_CURRENT_USER\Software\Classes\.pub"
New-Item -Path $pubKey -Force | Out-Null
Set-Item -Path $pubKey -Value $ForeignPubDefault

$completedCycles = 0
$failedCycle = $null
$failure = $null

try {
    for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
        Invoke-Phase $cycle "preclean" {
            Invoke-Uninstall | Out-Null
            Assert-Invariants $false
            return @{ uninstall_entries = @(Get-ChapteraUninstallers).Count }
        } | Out-Null

        Invoke-Phase $cycle "install_a" {
            $code = Invoke-Installer $SetupA
            Assert-Invariants $true
            $smoke = Invoke-ReaderSmoke $true
            return @{
                installer_exit_code = $code
                reader_sha256 = Get-FileSha256 (Join-Path $InstallRoot "current\chaptera-reader.exe")
                smoke = $smoke
            }
        } | Out-Null

        Invoke-Phase $cycle "update_then_bad_candidate_rollback" {
            $env:CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT = $InstallRoot
            $env:CHAPTERA_UPDATE_ACCEPT_PUB = $Fixture
            $env:CHAPTERA_UPDATE_ACCEPT_EXTERNAL_STATE = $externalState
            & cargo test -p chaptera-update-orchestrator --test windows_product_acceptance real_installed_reader_update_rollback_cycle -- --exact --nocapture
            $cargoExit = $LASTEXITCODE
            if ($cargoExit -ne 0) {
                throw "real installed update/rollback acceptance failed with cargo exit $cargoExit"
            }
            Assert-Invariants $true "B"
            $smoke = Invoke-ReaderSmoke $true
            $script:expectedReaderHashAfterRollback = Get-FileSha256 (Join-Path $InstallRoot "current\chaptera-reader.exe")
            return @{
                cargo_exit_code = $cargoExit
                confirmed_marker = "B"
                reader_sha256_after_rollback = $script:expectedReaderHashAfterRollback
                smoke_after_rollback = $smoke
            }
        } | Out-Null

        Invoke-Phase $cycle "inject_corruption_b" {
            $reader = Join-Path $InstallRoot "current\chaptera-reader.exe"
            [IO.File]::WriteAllBytes($reader, [Text.Encoding]::ASCII.GetBytes("CHAPTERA_SOAK_CORRUPTION"))
            $detection = Invoke-ReaderSmoke $false
            return @{
                corruption = "reader_binary_overwrite"
                corrupt_reader_sha256 = Get-FileSha256 $reader
                detected_by_smoke = -not $detection.success
                detection = $detection
            }
        } | Out-Null

        Invoke-Phase $cycle "repair_b_via_pinned_installer" {
            $code = Invoke-Installer $SetupB
            Assert-Invariants $true "B"
            $smoke = Invoke-ReaderSmoke $true
            $repairedHash = Get-FileSha256 (Join-Path $InstallRoot "current\chaptera-reader.exe")
            if ($repairedHash -ne $script:expectedReaderHashAfterRollback) {
                throw "repair did not restore Reader byte-identically: expected $($script:expectedReaderHashAfterRollback), got $repairedHash"
            }
            return @{
                installer_exit_code = $code
                repaired_reader_sha256 = $repairedHash
                restored_byte_identical_reader = $true
                smoke = $smoke
            }
        } | Out-Null

        Invoke-Phase $cycle "uninstall" {
            $code = Invoke-Uninstall
            Assert-Invariants $false
            return @{ uninstaller_exit_code = $code }
        } | Out-Null

        $completedCycles++
    }

    Save-State $Cycles "complete" "pass"
} catch {
    $failedCycle = [Math]::Min($completedCycles + 1, $Cycles)
    $failure = $_.Exception.Message
} finally {
    try {
        Invoke-Uninstall | Out-Null
    } catch {
        if (-not $failure) { $failure = "final cleanup failed: $($_.Exception.Message)" }
    }

    $eventLines = @()
    if (Test-Path -LiteralPath $eventsPath) {
        $eventLines = @(Get-Content -LiteralPath $eventsPath | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    }
    $events = @($eventLines | ForEach-Object { $_ | ConvertFrom-Json })
    $passEvents = @($events | Where-Object status -eq "pass")
    $failEvents = @($events | Where-Object status -eq "fail")
    $durations = @($passEvents | ForEach-Object { [double]$_.duration_ms } | Sort-Object)

    function Get-Percentile([double[]]$Values, [double]$P) {
        if ($Values.Count -eq 0) { return $null }
        $index = [Math]::Ceiling($P * $Values.Count) - 1
        if ($index -lt 0) { $index = 0 }
        if ($index -ge $Values.Count) { $index = $Values.Count - 1 }
        return [long]$Values[$index]
    }

    [ordered]@{
        schema_version = "chaptera.lifecycle-soak.summary.v1"
        run_id = $runId
        started_at_utc = $startedAt.ToString("o")
        finished_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        requested_cycles = $Cycles
        completed_cycles = $completedCycles
        failed_cycle = $failedCycle
        overall_status = if ($failure) { "fail" } else { "pass" }
        failure = $failure
        total_events = $events.Count
        pass_events = $passEvents.Count
        fail_events = $failEvents.Count
        duration_ms_p50 = Get-Percentile $durations 0.50
        duration_ms_p95 = Get-Percentile $durations 0.95
        fixture_sha256 = $FixtureSha256.ToLowerInvariant()
        external_state_sha256 = $externalHash
        final_registry = Get-RegistrySnapshot
        final_install_tree = Get-TreeSummary (Join-Path $InstallRoot "current")
    } | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 $summaryPath
}

if ($failure) {
    Write-Error "Chaptera lifecycle soak failed: $failure"
    exit 1
}

Write-Host "Chaptera lifecycle soak passed: $completedCycles/$Cycles cycles"
Write-Host "Summary: $summaryPath"
