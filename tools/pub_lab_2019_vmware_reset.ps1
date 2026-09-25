param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("preflight", "begin-revert", "finalize-revert")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$VmxPath,

    [string]$SnapshotName = "MODERN-2019-12527-GOLDEN-v1",

    [string]$ChallengeFile,

    [string]$EnvironmentManifest,

    [string]$ExpectedEnvironmentFingerprint,

    [string]$EvidenceOutput
)

$ErrorActionPreference = "Stop"

$BaselineId = "publisher-2019-build12527-golden-v1"
$VmRun = "C:\Program Files\VMware\VMware Workstation\vmrun.exe"
$VmwareVmx = "C:\Program Files\VMware\VMware Workstation\x64\vmware-vmx.exe"
$VdiskManager = "C:\Program Files\VMware\VMware Workstation\vmware-vdiskmanager.exe"

function Require-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-TextSha256([string]$Text) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Assert-Sha256([string]$Value, [string]$Label) {
    if ($Value -notmatch '^[0-9a-f]{64}$') {
        throw "$Label must be lowercase SHA-256"
    }
}

function Invoke-VmRun([string[]]$Arguments) {
    & $VmRun @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "vmrun failed ($LASTEXITCODE): $($Arguments -join ' ')"
    }
}

$VmRun = Require-File $VmRun "vmrun.exe"
$VmwareVmx = Require-File $VmwareVmx "vmware-vmx.exe"
$VdiskManager = Require-File $VdiskManager "vmware-vdiskmanager.exe"
$VmxPath = Require-File $VmxPath "target VMX"

$vmxHash = Get-Sha256 $VmxPath
$toolHashes = [ordered]@{
    vmrun_sha256 = Get-Sha256 $VmRun
    vmware_vmx_sha256 = Get-Sha256 $VmwareVmx
    vdiskmanager_sha256 = Get-Sha256 $VdiskManager
}

$vmxText = Get-Content -LiteralPath $VmxPath -Raw
if ($vmxText -notmatch '(?m)^displayName\s*=\s*"PUB-LAB-2019"\s*$') {
    throw "target VMX displayName is not PUB-LAB-2019"
}

if ($Mode -eq "preflight") {
    Invoke-VmRun @("-T", "ws", "list")
    Write-Host "PUB-LAB-2019 VMware preflight passed"
    exit 0
}

if ($SnapshotName -ne "MODERN-2019-12527-GOLDEN-v1") {
    throw "refusing non-authoritative snapshot: $SnapshotName"
}
if (-not $ChallengeFile) {
    throw "$Mode requires -ChallengeFile"
}

$challengePath = [IO.Path]::GetFullPath($ChallengeFile)

if ($Mode -eq "begin-revert") {
    if (-not $ExpectedEnvironmentFingerprint) {
        throw "begin-revert requires -ExpectedEnvironmentFingerprint"
    }
    $expectedFingerprint = $ExpectedEnvironmentFingerprint.ToLowerInvariant()
    Assert-Sha256 $expectedFingerprint "ExpectedEnvironmentFingerprint"

    $snapshots = @(& $VmRun -T ws listSnapshots $VmxPath)
    if ($LASTEXITCODE -ne 0) {
        throw "vmrun listSnapshots failed"
    }
    if (-not ($snapshots -contains $SnapshotName)) {
        throw "authoritative snapshot not found: $SnapshotName"
    }

    $running = @(& $VmRun -T ws list)
    if ($LASTEXITCODE -ne 0) {
        throw "vmrun list failed before restore"
    }
    $wasRunning = $running -contains $VmxPath

    $preState = [ordered]@{
        vmx_sha256 = $vmxHash
        was_running = [bool]$wasRunning
        snapshot_name = $SnapshotName
        snapshot_present = $true
        vmware = $toolHashes
    }
    $preStateHash = Get-TextSha256 ($preState | ConvertTo-Json -Depth 6 -Compress)

    if ($wasRunning) {
        Invoke-VmRun @("-T", "ws", "stop", $VmxPath, "hard")
    }

    $requestedAt = [DateTimeOffset]::UtcNow
    Invoke-VmRun @("-T", "ws", "revertToSnapshot", $VmxPath, $SnapshotName)

    $nonce = [guid]::NewGuid().ToString("N")
    $challenge = [ordered]@{
        schema_version = "chaptera.pub-lab-2019-restore-challenge.v1"
        vm_name = "PUB-LAB-2019"
        snapshot_name = $SnapshotName
        vmx_sha256 = $vmxHash
        restore_nonce = $nonce
        restore_requested_at_utc = $requestedAt.ToString("o")
        cold_start_succeeded_at_utc = $null
        expected_environment_fingerprint = $expectedFingerprint
        pre_restore_state_sha256 = $preStateHash
        vmware = $toolHashes
    }

    $challengeParent = Split-Path -Parent $challengePath
    if ($challengeParent) {
        New-Item -ItemType Directory -Force -Path $challengeParent | Out-Null
    }
    $challenge | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $challengePath -Encoding utf8

    Invoke-VmRun @("-T", "ws", "start", $VmxPath, "nogui")

    $challenge.cold_start_succeeded_at_utc = [DateTimeOffset]::UtcNow.ToString("o")
    $challenge | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $challengePath -Encoding utf8

    Write-Host "Cold restore started. Capture a fresh guest EnvironmentManifest bound to restore_nonce=$nonce, then run finalize-revert."
    exit 0
}

if (-not $EnvironmentManifest) {
    throw "finalize-revert requires -EnvironmentManifest"
}
if (-not $EvidenceOutput) {
    throw "finalize-revert requires -EvidenceOutput"
}

$challengePath = Require-File $challengePath "restore challenge"
$manifestPath = Require-File $EnvironmentManifest "post-restore EnvironmentManifest"
$challenge = Get-Content -LiteralPath $challengePath -Raw | ConvertFrom-Json
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json

if ($challenge.schema_version -ne "chaptera.pub-lab-2019-restore-challenge.v1") {
    throw "unsupported restore challenge schema"
}
if ($challenge.vm_name -ne "PUB-LAB-2019" -or $challenge.snapshot_name -ne $SnapshotName) {
    throw "restore challenge VM/snapshot identity mismatch"
}
if ($challenge.vmx_sha256 -ne $vmxHash) {
    throw "restore challenge VMX identity mismatch"
}
if (-not $challenge.cold_start_succeeded_at_utc) {
    throw "restore challenge does not prove a successful cold start"
}
if ($challenge.vmware.vmrun_sha256 -ne $toolHashes.vmrun_sha256 -or
    $challenge.vmware.vmware_vmx_sha256 -ne $toolHashes.vmware_vmx_sha256 -or
    $challenge.vmware.vdiskmanager_sha256 -ne $toolHashes.vdiskmanager_sha256) {
    throw "restore challenge VMware tool identity mismatch"
}
Assert-Sha256 ([string]$challenge.pre_restore_state_sha256) "restore challenge pre_restore_state_sha256"

$expectedFingerprint = ([string]$challenge.expected_environment_fingerprint).ToLowerInvariant()
Assert-Sha256 $expectedFingerprint "restore challenge expected environment fingerprint"

if ($manifest.schema_version -ne "chaptera.publisher2019-environment-manifest.v1") {
    throw "unsupported EnvironmentManifest schema"
}
if ($manifest.vm_name -ne "PUB-LAB-2019") {
    throw "EnvironmentManifest VM identity mismatch"
}
if ($manifest.restore_nonce -ne $challenge.restore_nonce) {
    throw "EnvironmentManifest is not bound to this restore challenge"
}
if ($manifest.publisher.version -ne "16.0" -or $manifest.publisher.build -ne "16.0.12527.22145") {
    throw "EnvironmentManifest Publisher build mismatch"
}
if ([int]$manifest.publisher.process_count -ne 0) {
    throw "EnvironmentManifest reports a stale Publisher process"
}
if ($manifest.environment_fingerprint -ne $expectedFingerprint) {
    throw "EnvironmentManifest environment fingerprint mismatch"
}

$started = [DateTimeOffset]::Parse([string]$challenge.cold_start_succeeded_at_utc)
$captured = [DateTimeOffset]::Parse([string]$manifest.captured_at_utc)
if ($captured -lt $started) {
    throw "EnvironmentManifest predates successful cold start"
}

$manifestHash = Get-Sha256 $manifestPath
$challengeHash = Get-Sha256 $challengePath
$completedAt = [DateTimeOffset]::UtcNow

$evidence = [ordered]@{
    schema_version = "chaptera.pub-lab-2019-vmware-evidence.v1"
    baseline_id = $BaselineId
    vm_identity = [ordered]@{
        name = "PUB-LAB-2019"
        config_fingerprint = $vmxHash
    }
    snapshot_identity = [ordered]@{
        name = $SnapshotName
        generation = 1
    }
    vmx_sha256 = $vmxHash
    environment_manifest_sha256 = $manifestHash
    environment_fingerprint = $manifest.environment_fingerprint
    vmware = $toolHashes
    restore = [ordered]@{
        challenge_sha256 = $challengeHash
        started_at_utc = [string]$challenge.restore_requested_at_utc
        completed_at_utc = $completedAt.ToString("o")
        pre_restore_state_sha256 = [string]$challenge.pre_restore_state_sha256
        post_restore_state_sha256 = $manifestHash
        revert_succeeded = $true
        cold_start_succeeded = $true
        post_boot_capture_bound = $true
        environment_match = $true
        restore_verified = $true
    }
    privacy = [ordered]@{
        local_paths_serialized = $false
        credentials_serialized = $false
        licensed_media_serialized = $false
        restore_nonce_serialized = $false
    }
}

$out = [IO.Path]::GetFullPath($EvidenceOutput)
$outParent = Split-Path -Parent $out
if ($outParent) {
    New-Item -ItemType Directory -Force -Path $outParent | Out-Null
}
$evidence | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath $out -Encoding utf8

Write-Host "Verified VMware backend evidence written: $out"
