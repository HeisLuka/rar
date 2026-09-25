param(
    [Parameter(Mandatory = $true)]
    [string]$BaselineId,

    [Parameter(Mandatory = $true)]
    [string]$SnapshotId,

    [Parameter(Mandatory = $true)]
    [string]$ExperimentId,

    [Parameter(Mandatory = $true)]
    [string]$PacketSha256,

    [Parameter(Mandatory = $true)]
    [string]$ReceiptPath
)

$ErrorActionPreference = "Stop"

$ExpectedBaseline = "publisher-2019-build12527-golden-v1"
$ExpectedSnapshot = "MODERN-2019-12527-GOLDEN-v1"

if ($BaselineId -ne $ExpectedBaseline) {
    throw "Unsupported PUB-LAB-2019 baseline: $BaselineId"
}
if ($SnapshotId -ne $ExpectedSnapshot) {
    throw "Unsupported PUB-LAB-2019 snapshot: $SnapshotId"
}

$packet = $PacketSha256.ToLowerInvariant()
if ($packet -notmatch '^[0-9a-f]{64}$') {
    throw "PacketSha256 must be lowercase SHA-256"
}

$vmxPath = [Environment]::ExpandEnvironmentVariables([string]$env:PUB_LAB_2019_VMX_PATH)
$expectedFingerprint = ([string]$env:PUB_LAB_2019_EXPECTED_ENVIRONMENT_FINGERPRINT).ToLowerInvariant()
$captureProvider = [Environment]::ExpandEnvironmentVariables([string]$env:PUB_LAB_2019_ENV_CAPTURE_PROVIDER)

if ([string]::IsNullOrWhiteSpace($vmxPath)) {
    throw "PUB_LAB_2019_VMX_PATH is required"
}
if ([string]::IsNullOrWhiteSpace($captureProvider)) {
    throw "PUB_LAB_2019_ENV_CAPTURE_PROVIDER is required"
}
if ($expectedFingerprint -notmatch '^[0-9a-f]{64}$') {
    throw "PUB_LAB_2019_EXPECTED_ENVIRONMENT_FINGERPRINT must be lowercase SHA-256"
}
if (-not (Test-Path -LiteralPath $captureProvider -PathType Leaf)) {
    throw "PUB_LAB_2019_ENV_CAPTURE_PROVIDER does not exist"
}

$resetAdapter = Join-Path $PSScriptRoot "pub_lab_2019_vmware_reset.ps1"
$evidenceValidator = Join-Path $PSScriptRoot "validate_pub_lab_2019_vmware_evidence.py"
$receiptBuilder = Join-Path $PSScriptRoot "build_pub_research_reset_receipt.py"
$receiptVerifier = Join-Path $PSScriptRoot "research-runner\verify_reset_receipt.py"

foreach ($required in @($resetAdapter, $evidenceValidator, $receiptBuilder, $receiptVerifier)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required reset component missing: $required"
    }
}

$out = [IO.Path]::GetFullPath($ReceiptPath)
$outParent = Split-Path -Parent $out
if ($outParent) {
    New-Item -ItemType Directory -Force -Path $outParent | Out-Null
}
Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("chaptera-pub-lab-reset-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null

$challenge = Join-Path $tempRoot "restore-challenge.json"
$manifest = Join-Path $tempRoot "environment-manifest.json"
$evidence = Join-Path $tempRoot "vmware-evidence.json"

try {
    & pwsh -NoProfile -File $resetAdapter -Mode begin-revert -VmxPath $vmxPath -SnapshotName $SnapshotId -ChallengeFile $challenge -ExpectedEnvironmentFingerprint $expectedFingerprint
    if ($LASTEXITCODE -ne 0) {
        throw "VMware begin-revert failed with exit code $LASTEXITCODE"
    }

    & pwsh -NoProfile -File $captureProvider -VmxPath $vmxPath -ChallengeFile $challenge -OutputManifest $manifest
    if ($LASTEXITCODE -ne 0) {
        throw "Environment capture provider failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
        throw "Environment capture provider did not emit a manifest"
    }

    & pwsh -NoProfile -File $resetAdapter -Mode finalize-revert -VmxPath $vmxPath -SnapshotName $SnapshotId -ChallengeFile $challenge -EnvironmentManifest $manifest -EvidenceOutput $evidence
    if ($LASTEXITCODE -ne 0) {
        throw "VMware finalize-revert failed with exit code $LASTEXITCODE"
    }

    python $evidenceValidator $evidence
    if ($LASTEXITCODE -ne 0) {
        throw "VMware backend evidence validation failed"
    }

    python $receiptBuilder --evidence $evidence --baseline-id $BaselineId --snapshot-id $SnapshotId --experiment-id $ExperimentId --packet-sha256 $packet --output $out
    if ($LASTEXITCODE -ne 0) {
        throw "provider-neutral reset receipt build failed"
    }

    python $receiptVerifier --receipt $out --expected-baseline $BaselineId --expected-snapshot $SnapshotId --expected-experiment $ExperimentId --expected-packet-sha256 $packet
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue
        throw "provider-neutral reset receipt verification failed"
    }
}
catch {
    Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue
    throw
}
finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
