param(
    [Parameter(Mandatory = $true)]
    [string]$VmxPath,

    [Parameter(Mandatory = $true)]
    [string]$ChallengeFile,

    [Parameter(Mandatory = $true)]
    [string]$OutputManifest
)

$ErrorActionPreference = "Stop"

$VmRun = "C:\Program Files\VMware\VMware Workstation\vmrun.exe"
$GuestPowerShell = "C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
$guestUser = [string]$env:PUB_LAB_2019_GUEST_USER
$guestPassword = [string]$env:PUB_LAB_2019_GUEST_PASSWORD

function Require-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label missing: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Invoke-GuestVmRun([string]$Command, [string[]]$Arguments) {
    & $VmRun -T ws -gu $guestUser -gp $guestPassword $Command $VmxPath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "vmrun guest operation failed: $Command"
    }
}

if ([string]::IsNullOrWhiteSpace($guestUser)) {
    throw "PUB_LAB_2019_GUEST_USER is required"
}
if ([string]::IsNullOrWhiteSpace($guestPassword)) {
    throw "PUB_LAB_2019_GUEST_PASSWORD is required"
}

$VmRun = Require-File $VmRun "vmrun.exe"
$VmxPath = Require-File $VmxPath "target VMX"
$ChallengeFile = Require-File $ChallengeFile "restore challenge"

$challenge = Get-Content -LiteralPath $ChallengeFile -Raw | ConvertFrom-Json
if ($challenge.schema_version -ne "chaptera.pub-lab-2019-restore-challenge.v1" -or
    $challenge.vm_name -ne "PUB-LAB-2019" -or
    [string]$challenge.restore_nonce -notmatch '^[0-9a-f]{32}$') {
    throw "restore challenge identity is invalid"
}

$probe = Require-File (Join-Path $PSScriptRoot "pub_lab_2019_capture_environment_guest.ps1") "guest capture probe"
$finalizer = Require-File (Join-Path $PSScriptRoot "finalize_pub_lab_2019_environment_manifest.py") "environment finalizer"

$nonce = [string]$challenge.restore_nonce
$guestRoot = "C:\ProgramData\Chaptera\pub-lab-capture\$nonce"
$guestChallenge = "$guestRoot\restore-challenge.json"
$guestProbe = "$guestRoot\capture-environment.ps1"
$guestRaw = "$guestRoot\raw-environment.json"

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("chaptera-pub-lab-capture-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
$rawHost = Join-Path $tempRoot "raw-environment.json"

try {
    $ready = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        & $VmRun -T ws -gu $guestUser -gp $guestPassword listProcessesInGuest $VmxPath *> $null
        if ($LASTEXITCODE -eq 0) {
            $ready = $true
            break
        }
        Start-Sleep -Seconds 2
    }
    if (-not $ready) {
        throw "VMware Tools guest operations did not become ready"
    }

    Invoke-GuestVmRun "runProgramInGuest" @("C:\\Windows\\System32\\cmd.exe", "/c", "mkdir $guestRoot")
    Invoke-GuestVmRun "copyFileFromHostToGuest" @($ChallengeFile, $guestChallenge)
    Invoke-GuestVmRun "copyFileFromHostToGuest" @($probe, $guestProbe)
    Invoke-GuestVmRun "runProgramInGuest" @(
        $GuestPowerShell,
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", $guestProbe,
        "-ChallengeFile", $guestChallenge,
        "-OutputRawManifest", $guestRaw
    )
    Invoke-GuestVmRun "copyFileFromGuestToHost" @($guestRaw, $rawHost)

    if (-not (Test-Path -LiteralPath $rawHost -PathType Leaf)) {
        throw "guest capture did not return a raw EnvironmentManifest"
    }

    $out = [IO.Path]::GetFullPath($OutputManifest)
    $parent = Split-Path -Parent $out
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }

    python $finalizer --challenge $ChallengeFile --raw $rawHost --output $out
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue
        throw "canonical EnvironmentManifest finalization failed"
    }
}
finally {
    & $VmRun -T ws -gu $guestUser -gp $guestPassword runProgramInGuest $VmxPath "C:\Windows\System32\cmd.exe" "/c" "rmdir /s /q $guestRoot" *> $null
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
