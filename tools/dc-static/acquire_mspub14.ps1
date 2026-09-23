param(
    [string]$OutRoot = "out/dc-static-01"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$packageUrl = "https://download.microsoft.com/download/0/5/e/05e96d4b-4837-4775-a9b8-7bd185d9d524/publisher2010-kb3114395-fullfile-x86-glb.exe"
$packageSha256 = "9bf8c3771b133781b29ffe24a913d62fd9a009a8c5da46a80e192805759063d8"
$targetSha256 = "27c00f7f06957f24d392f9c61fbd3b40282f15dc595caf66785b561f559d7b97"
$targetSize = 9675944

$root = New-Item -ItemType Directory -Force -Path $OutRoot
$acq = New-Item -ItemType Directory -Force -Path (Join-Path $root "acquisition")
$layers = New-Item -ItemType Directory -Force -Path (Join-Path $root "layers")
$private = New-Item -ItemType Directory -Force -Path (Join-Path $root "private")
$public = New-Item -ItemType Directory -Force -Path (Join-Path $root "public")

$package = Join-Path $acq "publisher2010-kb3114395-fullfile-x86-glb.exe"
Invoke-WebRequest -Uri $packageUrl -OutFile $package

$actualPackageSha = (Get-FileHash $package -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualPackageSha -ne $packageSha256) {
    throw "Microsoft package SHA-256 mismatch: expected $packageSha256 got $actualPackageSha"
}

$seven = (Get-Command 7z.exe -ErrorAction SilentlyContinue).Source
if (-not $seven) {
    $seven = "C:\Program Files\7-Zip\7z.exe"
}
if (-not (Test-Path $seven)) {
    throw "7z.exe is unavailable on runner"
}

function Test-ArchiveCandidate([System.IO.FileInfo]$File) {
    $ext = $File.Extension.ToLowerInvariant()
    if ($ext -in @(".msp", ".msi", ".cab")) { return $true }
    if ($File.Name -eq "PATCH_CAB") { return $true }

    try {
        $stream = [System.IO.File]::OpenRead($File.FullName)
        try {
            if ($stream.Length -ge 4) {
                $buf = New-Object byte[] 4
                [void]$stream.Read($buf, 0, 4)
                $sig = [System.Text.Encoding]::ASCII.GetString($buf)
                if ($sig -eq "MSCF") { return $true }
            }
        } finally {
            $stream.Dispose()
        }
    } catch {}

    return $false
}

$layer0 = New-Item -ItemType Directory -Force -Path (Join-Path $layers "00-sfx")
$nativeExtractWorked = $false
try {
    $proc = Start-Process -FilePath $package -ArgumentList @("/quiet", "/extract:$($layer0.FullName)") -Wait -PassThru
    $nativeExtractWorked = ($proc.ExitCode -eq 0 -and @(Get-ChildItem $layer0 -Force).Count -gt 0)
} catch {
    Write-Warning "Native Office self-extract failed: $($_.Exception.Message)"
}

if (-not $nativeExtractWorked) {
    & $seven x $package "-o$($layer0.FullName)" -y | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "7-Zip could not unpack the Microsoft SFX"
    }
}

$queue = [System.Collections.Generic.Queue[object]]::new()
Get-ChildItem $layer0 -Recurse -File | ForEach-Object {
    if (Test-ArchiveCandidate $_) {
        $queue.Enqueue([pscustomobject]@{ File = $_; Depth = 1 })
    }
}

$seen = @{}
$extractIndex = 1
$patchCabCount = 0
$cabSignatureCount = 0
while ($queue.Count -gt 0) {
    $item = $queue.Dequeue()
    if ($item.Depth -gt 5) { continue }

    $hash = (Get-FileHash $item.File.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($seen.ContainsKey($hash)) { continue }
    $seen[$hash] = $true

    if ($item.File.Name -eq "PATCH_CAB") { $patchCabCount += 1 }
    try {
        $s = [System.IO.File]::OpenRead($item.File.FullName)
        try {
            if ($s.Length -ge 4) {
                $b = New-Object byte[] 4
                [void]$s.Read($b, 0, 4)
                if ([System.Text.Encoding]::ASCII.GetString($b) -eq "MSCF") {
                    $cabSignatureCount += 1
                }
            }
        } finally { $s.Dispose() }
    } catch {}

    $safeBase = ($item.File.BaseName -replace '[^A-Za-z0-9._-]', '_')
    if (-not $safeBase) { $safeBase = "container" }
    $dest = New-Item -ItemType Directory -Force -Path (Join-Path $layers ("{0:D2}-{1}" -f $extractIndex, $safeBase))
    $extractIndex += 1

    & $seven x $item.File.FullName "-o$($dest.FullName)" -y | Out-Host
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "7-Zip could not expand $($item.File.FullName)"
        continue
    }

    Get-ChildItem $dest -Recurse -File | ForEach-Object {
        if (Test-ArchiveCandidate $_) {
            $queue.Enqueue([pscustomobject]@{ File = $_; Depth = $item.Depth + 1 })
        }
    }
}

$inventory = @()
$target = $null
Get-ChildItem $layers -Recurse -File | ForEach-Object {
    $row = [ordered]@{
        path = $_.FullName.Substring($layers.FullName.Length).TrimStart("\")
        size = $_.Length
        sha256 = $null
        file_version = $null
        product_version = $null
    }

    if ($_.Length -eq $targetSize -or $_.Name -match "^(mspub|morph9|prtf9|ptxt9|pubconv|pubtrap)\.") {
        $row.sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        try {
            $vi = (Get-Item $_.FullName).VersionInfo
            $row.file_version = $vi.FileVersion
            $row.product_version = $vi.ProductVersion
        } catch {}

        if ($row.sha256 -eq $targetSha256) {
            $target = $_
        }
    }
    $inventory += [pscustomobject]$row
}

$inventory | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 (Join-Path $public "acquisition-inventory.json")
@{
    schema = "dc-static-acquisition.v2"
    source_url = $packageUrl
    package_sha256 = $actualPackageSha
    expected_package_sha256 = $packageSha256
    target_expected_sha256 = $targetSha256
    target_expected_size = $targetSize
    native_sfx_extract = $nativeExtractWorked
    expanded_container_count = $seen.Count
    patch_cab_count = $patchCabCount
    cab_signature_count = $cabSignatureCount
    target_found = [bool]$target
} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $public "acquisition-summary.json")

if (-not $target) {
    Write-Error "Exact MSPUB.EXE target was not recovered after extensionless PATCH_CAB/MSCF expansion"
    exit 3
}

Copy-Item $target.FullName (Join-Path $private "mspub.exe")
Write-Host "Recovered exact MSPUB.EXE: $($target.FullName)"
Write-Host "SHA-256: $targetSha256"
