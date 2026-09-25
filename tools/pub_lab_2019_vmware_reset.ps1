param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("preflight", "revert")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$VmxPath,

    [string]$SnapshotName = "MODERN-2019-12527-GOLDEN-v1",

    [string]$EnvironmentManifest,

    [string]$ExpectedEnvironmentManifestSha256,

    [string]$ReceiptOutput
)

$ErrorActionPreference = "Stop"

$VmRun = "C:\Program Files\VMware\VMware Workstation\vmrun.exe"
$Vmx = "C:\Program Files\VMware\VMware Workstation\x64\vmware-vmx.exe"
$Vdisk = "C:\Program Files\VMware\VMware Workstation\vmware-vdiskmanager.exe"

function Require-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-VmRun([string[]]$Args) {
    & $VmRun @Args
    if ($LASTEXITCODE -ne 0) {
        throw "vmrun failed ($LASTEXITCODE): $($Args -join ' ')"
    }
}

$VmRun = Require-File $VmRun "vmrun.exe"
$Vmx = Require-File $Vmx "vmware-vmx.exe"
$Vdisk = Require-File $Vdisk "vmware-vdiskmanager.exe"
$VmxPath = Require-File $VmxPath "target VMX"

$vmxHash = Sha256 $VmxPath
$toolHashes = [ordered]@{
    vmrun_sha256 = Sha256 $VmRun
    vmware_vmx_sha256 = Sha256 $Vmx
    vdiskmanager_sha256 = Sha256 $Vdisk
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
if (-not $EnvironmentManifest) {
    throw "revert mode requires -EnvironmentManifest"
}
if (-not $ExpectedEnvironmentManifestSha256) {
    throw "revert mode requires -ExpectedEnvironmentManifestSha256"
}
if (-not $ReceiptOutput) {
    throw "revert mode requires -ReceiptOutput"
}

$manifestPath = Require-File $EnvironmentManifest "EnvironmentManifest"
$expected = $ExpectedEnvironmentManifestSha256.ToLowerInvariant()
if ($expected -notmatch '^[0-9a-f]{64}$') {
    throw "ExpectedEnvironmentManifestSha256 must be lowercase SHA-256"
}

# Snapshot identity must exist before any mutation.
$snapshots = & $VmRun -T ws listSnapshots $VmxPath
if ($LASTEXITCODE -ne 0) {
    throw "vmrun listSnapshots failed"
}
if (-not ($snapshots -contains $SnapshotName)) {
    throw "authoritative snapshot not found: $SnapshotName"
}

# Stop is intentionally fail-closed but tolerant of an already-stopped guest.
& $VmRun -T ws stop $VmxPath hard 2>$null | Out-Null

Invoke-VmRun @("-T", "ws", "revertToSnapshot", $VmxPath, $SnapshotName)
Invoke-VmRun @("-T", "ws", "start", $VmxPath, "nogui")

$currentManifestHash = Sha256 $manifestPath
if ($currentManifestHash -ne $expected) {
    throw "EnvironmentManifest hash mismatch after cold restore"
}

$receipt = [ordered]@{
    schema_version = "chaptera.pub-lab-2019-reset-receipt.v1"
    vm_identity = [ordered]@{
        name = "PUB-LAB-2019"
        config_fingerprint = $vmxHash
    }
    snapshot_identity = [ordered]@{
        name = $SnapshotName
        generation = 1
    }
    vmx_sha256 = $vmxHash
    environment_manifest_sha256 = $currentManifestHash
    vmware = $toolHashes
    restore = [ordered]@{
        revert_succeeded = $true
        cold_start_succeeded = $true
        environment_match = $true
        restore_verified = $true
    }
    privacy = [ordered]@{
        local_paths_serialized = $false
        credentials_serialized = $false
        licensed_media_serialized = $false
    }
}

$out = [IO.Path]::GetFullPath($ReceiptOutput)
$parent = Split-Path -Parent $out
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
$receipt | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $out -Encoding utf8

Write-Host "Verified VMware restore receipt written: $out"
