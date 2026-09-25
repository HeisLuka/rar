param(
    [Parameter(Mandatory = $true)]
    [string]$ChallengeFile,

    [Parameter(Mandatory = $true)]
    [string]$OutputRawManifest
)

$ErrorActionPreference = "Stop"

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

function Resolve-Mspub {
    $registryPaths = @(
        "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\MSPUB.EXE",
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\MSPUB.EXE",
        "HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\MSPUB.EXE"
    )
    foreach ($key in $registryPaths) {
        if (Test-Path $key) {
            $candidate = (Get-Item $key).GetValue("")
            if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
                return (Resolve-Path -LiteralPath $candidate).Path
            }
        }
    }

    $fallbacks = @()
    if ($env:ProgramFiles) {
        $fallbacks += (Join-Path $env:ProgramFiles "Microsoft Office\root\Office16\MSPUB.EXE")
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    if ($programFilesX86) {
        $fallbacks += (Join-Path $programFilesX86 "Microsoft Office\root\Office16\MSPUB.EXE")
    }
    $existing = @($fallbacks | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -Unique)
    if ($existing.Count -ne 1) {
        throw "Could not resolve one authoritative MSPUB.EXE"
    }
    return (Resolve-Path -LiteralPath $existing[0]).Path
}

function Resolve-OfficeModule([string]$MspubPath, [string]$Name) {
    $office16 = Split-Path -Parent $MspubPath
    $direct = Join-Path $office16 $Name
    if (Test-Path -LiteralPath $direct -PathType Leaf) {
        return (Resolve-Path -LiteralPath $direct).Path
    }

    $root = Split-Path -Parent $office16
    $matches = @(Get-ChildItem -LiteralPath $root -Recurse -File -Filter $Name -ErrorAction SilentlyContinue |
        ForEach-Object { $_.FullName } |
        Select-Object -Unique)
    if ($matches.Count -ne 1) {
        throw "Expected exactly one active $Name under Office root; found $($matches.Count)"
    }
    return $matches[0]
}

function Get-PeBitness([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        $stream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset + 4
        $machine = $reader.ReadUInt16()
        switch ($machine) {
            0x014c { return "x86" }
            0x8664 { return "x64" }
            default { throw ("Unsupported MSPUB PE machine 0x{0:x4}" -f $machine) }
        }
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Get-FontSetSha256 {
    $fontRoot = Join-Path $env:WINDIR "Fonts"
    $files = @(Get-ChildItem -LiteralPath $fontRoot -File -ErrorAction Stop | Sort-Object Name, Length)
    if ($files.Count -eq 0) {
        throw "Windows font set is empty"
    }
    $lines = foreach ($font in $files) {
        "{0}|{1}|{2}" -f $font.Name.ToLowerInvariant(), $font.Length, (Get-Sha256 $font.FullName)
    }
    return Get-TextSha256 ([string]::Join([Environment]::NewLine, $lines))
}

$challengePath = (Resolve-Path -LiteralPath $ChallengeFile).Path
$challenge = Get-Content -LiteralPath $challengePath -Raw | ConvertFrom-Json
if ($challenge.schema_version -ne "chaptera.pub-lab-2019-restore-challenge.v1") {
    throw "Unsupported restore challenge schema"
}
if ($challenge.vm_name -ne "PUB-LAB-2019") {
    throw "Restore challenge VM identity mismatch"
}
if ([string]$challenge.restore_nonce -notmatch '^[0-9a-f]{32}$') {
    throw "Restore challenge nonce is malformed"
}
if (-not $challenge.cold_start_succeeded_at_utc) {
    throw "Restore challenge does not contain successful cold-start time"
}

$publisherProcesses = @(Get-Process MSPUB -ErrorAction SilentlyContinue)
if ($publisherProcesses.Count -ne 0) {
    throw "MSPUB.EXE is running; refusing environment capture"
}

$mspub = Resolve-Mspub
$versionText = (Get-Item -LiteralPath $mspub).VersionInfo.FileVersion
if ([string]$versionText -notmatch '16\.0\.12527\.22145') {
    throw "MSPUB.EXE is not exact Publisher build 16.0.12527.22145"
}

$modules = [ordered]@{
    mspub_exe = Get-Sha256 $mspub
    oart_dll = Get-Sha256 (Resolve-OfficeModule $mspub "OART.DLL")
    wwlib_dll = Get-Sha256 (Resolve-OfficeModule $mspub "WWLIB.DLL")
    gfx_dll = Get-Sha256 (Resolve-OfficeModule $mspub "GFX.DLL")
    mspub_tlb = Get-Sha256 (Resolve-OfficeModule $mspub "MSPUB.TLB")
}

$printer = @(Get-CimInstance Win32_Printer -ErrorAction Stop | Where-Object { $_.Default })
if ($printer.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$printer[0].Name)) {
    throw "Expected exactly one pinned default printer"
}

$acpText = (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Nls\CodePage" -Name ACP -ErrorAction Stop).ACP
$acp = 0
if (-not [int]::TryParse([string]$acpText, [ref]$acp) -or $acp -le 0 -or $acp -gt 65535) {
    throw "System ANSI code page is invalid: $acpText"
}

$raw = [ordered]@{
    schema_version = "chaptera.publisher2019-environment-capture-raw.v1"
    vm_name = "PUB-LAB-2019"
    restore_nonce = [string]$challenge.restore_nonce
    captured_at_utc = [DateTimeOffset]::UtcNow.ToString("o")
    os_build = [Environment]::OSVersion.Version.ToString()
    system_locale = (Get-WinSystemLocale).Name
    user_locale = (Get-Culture).Name
    code_page = $acp
    time_zone = (Get-TimeZone).Id
    default_printer = [string]$printer[0].Name
    font_set_sha256 = Get-FontSetSha256
    publisher = [ordered]@{
        version = "16.0"
        build = "16.0.12527.22145"
        bitness = Get-PeBitness $mspub
        process_count = 0
        modules = $modules
    }
}

$out = [IO.Path]::GetFullPath($OutputRawManifest)
$parent = Split-Path -Parent $out
if ($parent) {
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
}
$raw | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath $out -Encoding utf8
Write-Host "Publisher 2019 raw environment capture written: $out"
