param(
    [ValidateSet("SelfTest","Prepare","Resume","Cleanup")]
    [string]$Mode = "SelfTest",

    [string]$InstallRoot,
    [string]$CandidateSource,
    [ValidateSet("prepared","previous-retained","candidate-activated")]
    [string]$FaultPhase = "candidate-activated",
    [string]$UpdaterRelativePath = "chaptera-reader.exe",

    [string]$StateRoot = (Join-Path $env:LOCALAPPDATA "Chaptera\updater-reboot-resume"),
    [string]$TaskName = "Chaptera Updater Reboot Resume",
    [string]$ProbeExe = "",

    [switch]$AcknowledgeProductMutation,
    [switch]$RequestReboot,
    [switch]$AcknowledgeSafeReboot,
    [switch]$AllowSameBootForSelfTest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Schema = "chaptera.local-updater-reboot-resume.v1"
$ScriptPath = [IO.Path]::GetFullPath($MyInvocation.MyCommand.Path)
$StateRoot = [IO.Path]::GetFullPath($StateRoot)
$StatePath = Join-Path $StateRoot "state.json"
$ReceiptPath = Join-Path $StateRoot "receipt.json"
$WrapperPath = Join-Path $StateRoot "resume.cmd"

function Resolve-Probe {
    if ($ProbeExe) {
        $p = [IO.Path]::GetFullPath($ProbeExe)
    } else {
        $p = [IO.Path]::GetFullPath("target\debug\chaptera-update-recovery-probe.exe")
    }
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) {
        throw "recovery probe missing: $p"
    }
    return $p
}

function Get-BootIdentity {
    return (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString("o")
}

function Write-DurableJson([string]$Path,[object]$Value) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    $next = "$Path.next"
    Remove-Item -Force $next -ErrorAction SilentlyContinue
    $json = $Value | ConvertTo-Json -Depth 12
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json + [Environment]::NewLine)
    $stream = [IO.FileStream]::new($next,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try {
        $stream.Write($bytes,0,$bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
    if (Test-Path -LiteralPath $Path) {
        [IO.File]::Replace($next,$Path,$null,$true)
    } else {
        [IO.File]::Move($next,$Path)
    }
}

function Parse-KeyValue([string[]]$Lines) {
    $map = @{}
    foreach ($line in $Lines) {
        $i = $line.IndexOf("=")
        if ($i -gt 0) {
            $map[$line.Substring(0,$i)] = $line.Substring($i + 1)
        }
    }
    return $map
}

function Invoke-Probe([string[]]$Arguments) {
    $probe = Resolve-Probe
    $lines = @(& $probe @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "recovery probe failed: $($Arguments -join ' ')"
    }
    return Parse-KeyValue $lines
}

function Remove-ResumeTask {
    & schtasks.exe /Delete /TN $TaskName /F 2>$null | Out-Null
    $global:LASTEXITCODE = 0
}

function Cleanup-State([switch]$KeepReceipt) {
    Remove-ResumeTask
    foreach ($p in @($StatePath,$WrapperPath)) {
        Remove-Item -Force -LiteralPath $p -ErrorAction SilentlyContinue
        Remove-Item -Force -LiteralPath "$p.next" -ErrorAction SilentlyContinue
    }
    if (-not $KeepReceipt) {
        Remove-Item -Force -LiteralPath $ReceiptPath -ErrorAction SilentlyContinue
    }
}

function Register-ResumeTask([string]$ResolvedInstallRoot,[string]$ResolvedProbe) {
    @"
@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$ScriptPath" -Mode Resume -InstallRoot "$ResolvedInstallRoot" -StateRoot "$StateRoot" -TaskName "$TaskName" -ProbeExe "$ResolvedProbe"
"@ | Set-Content -Encoding ascii $WrapperPath
    $taskCommand = '"' + $WrapperPath + '"'
    & schtasks.exe /Create /TN $TaskName /TR $taskCommand /SC ONLOGON /RL LIMITED /F | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "failed to register updater resume task" }
}

function Complete-Recovery([switch]$SameBootSelfTest) {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        throw "updater reboot state missing"
    }
    $state = Get-Content -Raw -LiteralPath $StatePath | ConvertFrom-Json
    if ($state.schema_version -ne $Schema) { throw "unsupported state schema" }

    $bootAfter = Get-BootIdentity
    $observedBootChange = $bootAfter -ne $state.boot_before_utc
    if (-not $SameBootSelfTest -and -not $observedBootChange) {
        throw "resume invoked but real Windows reboot was not observed"
    }

    $before = Invoke-Probe @("inspect",$state.install_root)
    if ($before["active"] -ne "true") { throw "expected active update journal before recovery" }
    if ($before["transaction_id"] -ne $state.transaction_id) {
        throw "journal transaction mismatch before recovery"
    }
    if ($before["phase"] -ne $state.fault_phase.Replace("-","_")) {
        throw "journal phase mismatch before recovery: $($before['phase'])"
    }

    $recovered = Invoke-Probe @("recover",$state.install_root)
    $expectedOutcome = if ($state.fault_phase -eq "prepared") {
        "prepared_transaction_aborted"
    } else {
        "unconfirmed_candidate_rolled_back"
    }
    if ($recovered["recovery_outcome"] -ne $expectedOutcome) {
        throw "unexpected recovery outcome: $($recovered['recovery_outcome'])"
    }
    if ($recovered["active"] -ne "false") { throw "journal remained active after recovery" }
    if ($recovered["current_tree_sha256"] -ne $state.expected_current_tree_sha256) {
        throw "recovered current tree does not match pre-fault predecessor"
    }

    $receipt = [ordered]@{
        schema_version = $Schema
        run_id = $state.run_id
        status = if ($SameBootSelfTest) { "SELFTEST_PASS" } else { "PASS" }
        transaction_id = $state.transaction_id
        fault_phase = $state.fault_phase
        expected_recovery_outcome = $expectedOutcome
        recovery_outcome = $recovered["recovery_outcome"]
        predecessor_tree_sha256 = $state.expected_current_tree_sha256
        recovered_tree_sha256 = $recovered["current_tree_sha256"]
        real_reboot_observed = if ($SameBootSelfTest) { $false } else { $true }
        observed_boot_identity_change = $observedBootChange
        completed_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    }
    Write-DurableJson $ReceiptPath $receipt
    Cleanup-State -KeepReceipt
    Write-Host "CHAPTERA_UPDATER_REBOOT_RESUME $($receipt.status)"
}

function Prepare-Fault([switch]$SelfTest) {
    if (-not $InstallRoot) { throw "InstallRoot is required" }
    if (-not $CandidateSource) { throw "CandidateSource is required" }

    $root = [IO.Path]::GetFullPath($InstallRoot)
    $candidate = [IO.Path]::GetFullPath($CandidateSource)
    if (-not (Test-Path -LiteralPath (Join-Path $root "current") -PathType Container)) {
        throw "current tree missing: $root"
    }
    if (-not (Test-Path -LiteralPath $candidate -PathType Container)) {
        throw "candidate tree missing: $candidate"
    }

    if (-not $SelfTest -and -not $AcknowledgeProductMutation) {
        $receipt = [ordered]@{
            schema_version = $Schema
            status = "MUTATION_DEFERRED_SAFETY"
            real_reboot_observed = $false
            product_tree_mutated = $false
            note = "Pass -AcknowledgeProductMutation only on the dedicated owner dogfood installation."
        }
        Write-DurableJson $ReceiptPath $receipt
        Write-Host "CHAPTERA_UPDATER_REBOOT_RESUME MUTATION_DEFERRED_SAFETY"
        return $false
    }

    Cleanup-State -KeepReceipt
    New-Item -ItemType Directory -Force -Path $StateRoot | Out-Null
    $probe = Resolve-Probe
    $before = Invoke-Probe @("inspect",$root)
    if ($before["active"] -eq "true") { throw "install already has an active update transaction" }
    $preHash = $before["current_tree_sha256"]
    if (-not $preHash) { throw "pre-fault current tree fingerprint missing" }

    $runId = [guid]::NewGuid().ToString("N")
    $tx = "reboot-$($FaultPhase.Replace('-','_'))-$($runId.Substring(0,12))"
    $prepared = Invoke-Probe @(
        "prepare-fault",$root,$candidate,$tx,"reboot-preflight-candidate",
        $UpdaterRelativePath,$FaultPhase
    )
    if ($prepared["transaction_id"] -ne $tx) { throw "prepared transaction mismatch" }

    $state = [ordered]@{
        schema_version = $Schema
        run_id = $runId
        prepared_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        boot_before_utc = Get-BootIdentity
        install_root = $root
        transaction_id = $tx
        fault_phase = $FaultPhase
        expected_current_tree_sha256 = $preHash
        probe_exe = $probe
    }
    Write-DurableJson $StatePath $state

    if ($SelfTest) {
        Complete-Recovery -SameBootSelfTest
        return $true
    }

    if (-not $RequestReboot -or -not $AcknowledgeSafeReboot) {
        # Never strand the owner install in a fault state merely because reboot
        # was not explicitly authorized. Recover immediately through product
        # logic and report the bounded result.
        $recovered = Invoke-Probe @("recover",$root)
        if ($recovered["current_tree_sha256"] -ne $preHash) {
            throw "deferred reboot recovery did not restore predecessor"
        }
        $receipt = [ordered]@{
            schema_version = $Schema
            run_id = $runId
            status = "REBOOT_DEFERRED_SAFETY"
            transaction_id = $tx
            fault_phase = $FaultPhase
            product_tree_mutated = $true
            product_tree_recovered = $true
            real_reboot_observed = $false
            recovery_outcome = $recovered["recovery_outcome"]
            recovered_tree_sha256 = $recovered["current_tree_sha256"]
        }
        Write-DurableJson $ReceiptPath $receipt
        Cleanup-State -KeepReceipt
        Write-Host "CHAPTERA_UPDATER_REBOOT_RESUME REBOOT_DEFERRED_SAFETY"
        return $false
    }

    Register-ResumeTask $root $probe
    & shutdown.exe /r /t 10 /d p:0:0 /c "Chaptera updater journal reboot-resume dogfood"
    if ($LASTEXITCODE -ne 0) { throw "Windows rejected reboot request: $LASTEXITCODE" }
    Write-Host "Chaptera updater reboot requested without /f; resume task is armed."
    return $true
}

switch ($Mode) {
    "SelfTest" {
        if (-not $InstallRoot -or -not $CandidateSource) {
            throw "SelfTest requires InstallRoot and CandidateSource"
        }
        Prepare-Fault -SelfTest | Out-Null
        $receipt = Get-Content -Raw -LiteralPath $ReceiptPath | ConvertFrom-Json
        if ($receipt.status -ne "SELFTEST_PASS" -or $receipt.real_reboot_observed) {
            throw "self-test receipt is invalid"
        }
        Write-Host "CHAPTERA_UPDATER_REBOOT_RESUME_SELFTEST PASS"
    }
    "Prepare" {
        Prepare-Fault | Out-Null
    }
    "Resume" {
        Complete-Recovery -SameBootSelfTest:$AllowSameBootForSelfTest
    }
    "Cleanup" {
        Cleanup-State
    }
}
