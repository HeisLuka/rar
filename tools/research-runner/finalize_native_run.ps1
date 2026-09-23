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

$packet = Get-Content -LiteralPath $PacketPath -Raw | ConvertFrom-Json
$root = (Resolve-Path -LiteralPath $OutputRoot).Path

$requiredEvidence = @($packet.evidence.required)
if ($null -ne $packet.reset -and [bool]$packet.reset.required) {
    $resetReceiptRelative = "analysis/reset-receipt.json"
    if ($requiredEvidence -notcontains $resetReceiptRelative) {
        $requiredEvidence += $resetReceiptRelative
    }
}

$missing = @()
foreach ($relative in $requiredEvidence) {
    $candidate = Join-Path $root ([string]$relative)
    if (-not (Test-Path -LiteralPath $candidate)) {
        $missing += [string]$relative
    }
}
if ($missing.Count -gt 0) {
    throw "Required evidence is missing: $($missing -join ', ')"
}

$forbiddenExtensions = @(
    ".pub", ".exe", ".dll", ".com", ".msi", ".msp", ".cab", ".iso",
    ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx"
)
foreach ($publicDirName in @("logs", "analysis")) {
    $publicDir = Join-Path $root $publicDirName
    if (Test-Path -LiteralPath $publicDir) {
        foreach ($file in Get-ChildItem -LiteralPath $publicDir -File -Recurse) {
            if ($forbiddenExtensions -contains $file.Extension.ToLowerInvariant()) {
                throw "Forbidden binary/document in upload-safe directory: $($file.FullName)"
            }
        }
    }
}

$records = @()
foreach ($file in Get-ChildItem -LiteralPath $root -File -Recurse | Sort-Object FullName) {
    $relative = [IO.Path]::GetRelativePath($root, $file.FullName).Replace("\", "/")
    if ($relative -eq "evidence-manifest.json") { continue }
    $visibility = if (
        $relative.StartsWith("logs/") -or
        $relative.StartsWith("analysis/") -or
        $relative -eq "environment.json"
    ) {
        "upload-safe"
    } else {
        "private-local"
    }
    $records += [ordered]@{
        path = $relative
        size = $file.Length
        sha256 = Get-Sha256 $file.FullName
        visibility = $visibility
    }
}

$manifest = [ordered]@{
    schema = "pub-research-evidence.v1"
    experiment_id = [string]$packet.id
    publisher_environment = [string]$packet.publisher_environment
    generated_utc = [DateTime]::UtcNow.ToString("o")
    required = @($requiredEvidence)
    files = $records
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $root "evidence-manifest.json") -Encoding UTF8
Write-Host "Evidence bundle validated: $($records.Count) files indexed."
