param(
    [ValidateSet("SelfTest","Prepare","Resume","Cleanup")]
    [string]$Mode = "SelfTest",

    [string]$StateRoot = (Join-Path $env:LOCALAPPDATA "Chaptera\dogfood-restart-preflight"),
    [string]$TaskName = "Chaptera Dogfood Restart Resume",

    [switch]$RequestReboot,
    [switch]$AcknowledgeSafeReboot,
    [switch]$AllowSameBootForSelfTest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Schema = "chaptera.local-restart-preflight.v1"
$ScriptPath = [IO.Path]::GetFullPath($MyInvocation.MyCommand.Path)
$StateRoot = [IO.Path]::GetFullPath($StateRoot)
$StatePath = Join-Path $StateRoot "restart-state.json"
$ReceiptPath = Join-Path $StateRoot "receipt.json"
$EventsPath = Join-Path $StateRoot "events.jsonl"
$WrapperPath = Join-Path $StateRoot "resume.cmd"
$LockTarget = Join-Path $StateRoot "lock-target.bin"
$LockReady = Join-Path $StateRoot "lock-ready.txt"
$LockHelper = Join-Path $StateRoot "hold-lock.ps1"

function Get-BootIdentity {
    $os = Get-CimInstance Win32_OperatingSystem
    return $os.LastBootUpTime.ToUniversalTime().ToString("o")
}

function Write-DurableJson([string]$Path, [object]$Value) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $next = "$Path.next"
    Remove-Item -Force $next -ErrorAction SilentlyContinue

    $json = $Value | ConvertTo-Json -Depth 12
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json + [Environment]::NewLine)
    $stream = [IO.FileStream]::new(
        $next,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }

    if (Test-Path -LiteralPath $Path) {
        [IO.File]::Replace($next, $Path, $null, $true)
    } else {
        [IO.File]::Move($next, $Path)
    }
}

function Write-Event([string]$Event, [hashtable]$Data = @{}) {
    New-Item -ItemType Directory -Force -Path $StateRoot | Out-Null
    $row = [ordered]@{
        schema_version = $Schema
        timestamp_utc = (Get-Date).ToUniversalTime().ToString("o")
        event = $Event
        data = $Data
    }
    ($row | ConvertTo-Json -Depth 8 -Compress) | Add-Content -Encoding utf8 $EventsPath
}

function Get-State {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        throw "restart state is missing"
    }
    return Get-Content -Raw -LiteralPath $StatePath | ConvertFrom-Json
}

function Remove-ResumeTask {
    & schtasks.exe /Delete /TN $TaskName /F 2>$null | Out-Null
    $global:LASTEXITCODE = 0
}

function Register-ResumeTask {
    New-Item -ItemType Directory -Force -Path $StateRoot | Out-Null
    @"
@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$ScriptPath" -Mode Resume -StateRoot "$StateRoot" -TaskName "$TaskName"
"@ | Set-Content -Encoding ascii $WrapperPath

    $taskCommand = '"' + $WrapperPath + '"'
    & schtasks.exe /Create /TN $TaskName /TR $taskCommand /SC ONLOGON /RL LIMITED /F | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "failed to register resume task: schtasks exit $LASTEXITCODE"
    }

    & schtasks.exe /Query /TN $TaskName | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "resume task query failed after registration"
    }

    Write-Event "resume_task_registered" @{
        scheduler = "windows_task_scheduler"
        trigger = "on_logon"
        run_level = "limited"
    }
}

function Run-OwnedProcessAndFileLockPrimitive {
    New-Item -ItemType Directory -Force -Path $StateRoot | Out-Null
    Remove-Item -Force $LockTarget,$LockReady,$LockHelper -ErrorAction SilentlyContinue
    [IO.File]::WriteAllBytes($LockTarget, [Text.Encoding]::ASCII.GetBytes("chaptera-lock-preflight"))

    @'
param([string]$Target,[string]$Ready)
$ErrorActionPreference = "Stop"
$stream = [IO.File]::Open($Target,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
try {
    Set-Content -Encoding ascii -Path $Ready -Value $PID
    while ($true) { Start-Sleep -Seconds 30 }
} finally {
    $stream.Dispose()
}
'@ | Set-Content -Encoding utf8 $LockHelper

    $child = Start-Process -FilePath "powershell.exe" -ArgumentList @(
        "-NoProfile",
        "-ExecutionPolicy","Bypass",
        "-File",$LockHelper,
        "-Target",$LockTarget,
        "-Ready",$LockReady
    ) -PassThru

    try {
        $deadline = (Get-Date).AddSeconds(15)
        while (-not (Test-Path -LiteralPath $LockReady)) {
            if ((Get-Date) -gt $deadline) {
                throw "owned lock helper did not become ready"
            }
            if ($child.HasExited) {
                throw "owned lock helper exited before taking lock: $($child.ExitCode)"
            }
            Start-Sleep -Milliseconds 100
            $child.Refresh()
        }

        $locked = $false
        try {
            $probe = [IO.File]::Open(
                $LockTarget,
                [IO.FileMode]::Open,
                [IO.FileAccess]::ReadWrite,
                [IO.FileShare]::None
            )
            $probe.Dispose()
        } catch [IO.IOException] {
            $locked = $true
        }

        if (-not $locked) {
            throw "exclusive file lock was not observed"
        }

        Write-Event "exclusive_file_lock_observed" @{
            owned_process = $true
            lock_mode = "FileShare.None"
        }

        Stop-Process -Id $child.Id -Force
        $child.WaitForExit()

        $probe = [IO.File]::Open(
            $LockTarget,
            [IO.FileMode]::Open,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::None
        )
        $probe.Dispose()

        Write-Event "owned_process_terminated_lock_released" @{
            termination = "Stop-Process -Force"
            post_kill_lock_open = "pass"
        }

        return [ordered]@{
            owned_process_kill = "pass"
            exclusive_file_lock = "pass"
            lock_release_after_process_exit = "pass"
        }
    } finally {
        if (-not $child.HasExited) {
            Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
        }
    }
}

function Cleanup-TemporaryState([switch]$KeepReceiptAndEvents) {
    Remove-ResumeTask
    foreach ($path in @($StatePath,$WrapperPath,$LockTarget,$LockReady,$LockHelper)) {
        Remove-Item -Force -LiteralPath $path -ErrorAction SilentlyContinue
        Remove-Item -Force -LiteralPath "$path.next" -ErrorAction SilentlyContinue
    }

    if (-not $KeepReceiptAndEvents) {
        Remove-Item -Force -LiteralPath $ReceiptPath,$EventsPath -ErrorAction SilentlyContinue
    }

    Write-Host "Chaptera restart preflight temporary state cleaned."
}

function Prepare-Restart([switch]$ForSelfTest) {
    Cleanup-TemporaryState -KeepReceiptAndEvents
    New-Item -ItemType Directory -Force -Path $StateRoot | Out-Null

    $runId = [guid]::NewGuid().ToString("N")
    $bootBefore = Get-BootIdentity
    $primitives = Run-OwnedProcessAndFileLockPrimitive

    $state = [ordered]@{
        schema_version = $Schema
        run_id = $runId
        status = "prepared"
        prepared_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        boot_before_utc = $bootBefore
        expected_next_mode = "Resume"
        scheduler = "windows_task_scheduler"
        trigger = "on_logon"
        selftest = [bool]$ForSelfTest
    }
    Write-DurableJson $StatePath $state
    $markerHash = (Get-FileHash -LiteralPath $StatePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Event "durable_restart_marker_written" @{
        state_sha256 = $markerHash
        boot_before_utc = $bootBefore
    }

    Register-ResumeTask

    return [ordered]@{
        run_id = $runId
        boot_before_utc = $bootBefore
        marker_sha256 = $markerHash
        primitives = $primitives
    }
}

function Complete-Resume([switch]$SameBootSelfTest) {
    $state = Get-State
    if ($state.schema_version -ne $Schema) {
        throw "unsupported restart-state schema: $($state.schema_version)"
    }
    if ($state.expected_next_mode -ne "Resume") {
        throw "restart state does not expect Resume"
    }

    $bootAfter = Get-BootIdentity
    $observedBootChange = $bootAfter -ne $state.boot_before_utc

    if ($SameBootSelfTest) {
        # Hosted/self-test mode is intentionally incapable of proving reboot.
        # Even if the platform reports a changed boot-time representation, do
        # not upgrade this receipt into a real-reboot claim.
        $bootChanged = $false
        $status = "SELFTEST_PASS"
    } else {
        $bootChanged = $observedBootChange
        if (-not $bootChanged) {
            throw "resume invoked but Windows reboot was not observed"
        }
        $status = "PASS"
    }

    $receipt = [ordered]@{
        schema_version = $Schema
        run_id = $state.run_id
        status = $status
        completed_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        owned_process_kill = "pass"
        exclusive_file_lock = "pass"
        lock_release = "pass"
        durable_restart_marker = "pass"
        resume_detection = "pass"
        scheduler = "windows_task_scheduler"
        scheduler_trigger = "on_logon"
        real_reboot_observed = $bootChanged
        reboot_deferred_for_safety = $false
        boot_identity_changed = $bootChanged
        observed_boot_identity_change = $observedBootChange
    }
    Write-DurableJson $ReceiptPath $receipt
    Write-Event "resume_detected" @{
        boot_identity_changed = $bootChanged
        result = $status
    }

    Cleanup-TemporaryState -KeepReceiptAndEvents

    Write-Host "Chaptera restart preflight result: $status"
    Write-Host "Receipt: $ReceiptPath"
}

switch ($Mode) {
    "SelfTest" {
        $prepared = Prepare-Restart -ForSelfTest
        Complete-Resume -SameBootSelfTest

        $receipt = Get-Content -Raw -LiteralPath $ReceiptPath | ConvertFrom-Json
        if ($receipt.status -ne "SELFTEST_PASS") {
            throw "self-test receipt was not SELFTEST_PASS"
        }
        if ($receipt.real_reboot_observed) {
            throw "self-test must not claim a real reboot"
        }
        & schtasks.exe /Query /TN $TaskName 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) {
            throw "resume task survived self-test cleanup"
        }
        $global:LASTEXITCODE = 0

        Write-Host "CHAPTERA_RESTART_PREFLIGHT_SELFTEST PASS"
        break
    }

    "Prepare" {
        $prepared = Prepare-Restart

        if (-not $RequestReboot -or -not $AcknowledgeSafeReboot) {
            $receipt = [ordered]@{
                schema_version = $Schema
                run_id = $prepared.run_id
                status = "REBOOT_DEFERRED_SAFETY"
                completed_at_utc = (Get-Date).ToUniversalTime().ToString("o")
                owned_process_kill = $prepared.primitives.owned_process_kill
                exclusive_file_lock = $prepared.primitives.exclusive_file_lock
                lock_release = $prepared.primitives.lock_release_after_process_exit
                durable_restart_marker = "pass"
                resume_task_registration = "pass"
                real_reboot_observed = $false
                reboot_deferred_for_safety = $true
                note = "No reboot was attempted. Re-run Prepare with both -RequestReboot and -AcknowledgeSafeReboot only when the interactive host is known safe to restart."
            }
            Write-DurableJson $ReceiptPath $receipt
            Write-Event "reboot_deferred_safety" @{
                request_reboot = [bool]$RequestReboot
                explicit_safe_ack = [bool]$AcknowledgeSafeReboot
            }
            Cleanup-TemporaryState -KeepReceiptAndEvents
            Write-Host "CHAPTERA_RESTART_PREFLIGHT REBOOT_DEFERRED_SAFETY"
            break
        }

        $state = Get-State
        $state.status = "reboot_requested"
        $state | Add-Member -NotePropertyName reboot_requested_at_utc -NotePropertyValue ((Get-Date).ToUniversalTime().ToString("o")) -Force
        Write-DurableJson $StatePath $state
        Write-Event "reboot_requested" @{
            explicit_safe_ack = $true
            shutdown_force_flag = $false
        }

        & shutdown.exe /r /t 10 /d p:0:0 /c "Chaptera restart-resume preflight"
        if ($LASTEXITCODE -ne 0) {
            throw "Windows rejected reboot request: shutdown exit $LASTEXITCODE"
        }

        Write-Host "Reboot requested without /f. Resume will run at next interactive logon."
        break
    }

    "Resume" {
        Complete-Resume -SameBootSelfTest:$AllowSameBootForSelfTest
        break
    }

    "Cleanup" {
        Cleanup-TemporaryState
        if (Test-Path -LiteralPath $StateRoot) {
            $remaining = @(Get-ChildItem -LiteralPath $StateRoot -Force -ErrorAction SilentlyContinue)
            if ($remaining.Count -eq 0) {
                Remove-Item -Force -LiteralPath $StateRoot -ErrorAction SilentlyContinue
            }
        }
        break
    }
}
