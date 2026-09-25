param(
    [Parameter(Mandatory = $true)]
    [string]$BundleDir,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot,
    [string]$ExpectedPublisherVersion = "16.0",
    [string]$ExpectedBuildPrefix = "12527"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$bundle = (Resolve-Path -LiteralPath $BundleDir).Path
New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
$output = (Resolve-Path -LiteralPath $OutputRoot).Path

Import-Module (Join-Path $repoRoot "tools\windows\pub-runtime\PubRuntime.psm1") -Force

$manifestPath = Join-Path $bundle "handoff.json"
$sourcePath = Join-Path $bundle "source.pub"
$candidatePath = Join-Path $bundle "candidate.pub"
foreach ($path in @($manifestPath, $sourcePath, $candidatePath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Handoff bundle is incomplete: $path"
    }
}

$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.schema -ne "chaptera.pub-native-story-handoff.v1") {
    throw "Unsupported handoff manifest schema: $($manifest.schema)"
}

$sourceBefore = Get-PubFileRecord $sourcePath
$candidateBefore = Get-PubFileRecord $candidatePath
if ($sourceBefore.sha256 -ne ([string]$manifest.source_sha256).ToLowerInvariant()) {
    throw "Source PUB hash does not match handoff manifest"
}
if ($candidateBefore.sha256 -ne ([string]$manifest.candidate_sha256).ToLowerInvariant()) {
    throw "Candidate PUB hash does not match handoff manifest"
}

$publisher = Get-PubPublisherIdentity
if (-not $publisher.available) {
    throw "Microsoft Publisher COM automation is unavailable"
}
if ($publisher.version.state -ne "value" -or [string]$publisher.version.value -ne $ExpectedPublisherVersion) {
    throw "Publisher version mismatch: expected $ExpectedPublisherVersion"
}
if ($publisher.build.state -ne "value" -or -not ([string]$publisher.build.value).StartsWith($ExpectedBuildPrefix)) {
    throw "Publisher build mismatch: expected prefix $ExpectedBuildPrefix, got $($publisher.build.value)"
}

function Close-PubDocument {
    param($Document)
    if ($null -eq $Document) { return }
    try { $Document.Close() } catch {}
    try { [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Document) } catch {}
}

function Get-SafeDocumentSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][string]$Phase
    )
    $shapeCount = 0
    for ($pageIndex = 1; $pageIndex -le [int]$Document.Pages.Count; $pageIndex++) {
        $shapeCount += [int]$Document.Pages.Item($pageIndex).Shapes.Count
    }
    return [ordered]@{
        phase = $Phase
        page_count = [int]$Document.Pages.Count
        shape_count = $shapeCount
    }
}

$savedPath = Join-Path $output "native-saveas.pub"
$receiptPath = Join-Path $output "native-roundtrip-receipt.json"
Remove-Item -LiteralPath $savedPath -Force -ErrorAction SilentlyContinue

$openSnapshot = $null
$reopenSnapshot = $null
$application = $null
$document = $null
try {
    $application = New-PubPublisherApplication
    $document = $application.Open($candidatePath, $false, $false)
    $openSnapshot = Get-SafeDocumentSnapshot -Document $document -Phase "candidate_open"
    # pbFilePublication = 1. Save to a new path; never overwrite the generated candidate.
    $document.SaveAs($savedPath, 1, $false)
}
finally {
    Close-PubDocument $document
    Close-PubPublisherApplication $application
}

if (-not (Test-Path -LiteralPath $savedPath -PathType Leaf)) {
    throw "Publisher SaveAs did not create native-saveas.pub"
}

$reopenApplication = $null
$reopenDocument = $null
try {
    $reopenApplication = New-PubPublisherApplication
    $reopenDocument = $reopenApplication.Open($savedPath, $true, $false)
    $reopenSnapshot = Get-SafeDocumentSnapshot -Document $reopenDocument -Phase "fresh_reopen"
}
finally {
    Close-PubDocument $reopenDocument
    Close-PubPublisherApplication $reopenApplication
}

$sourceAfter = Get-PubFileRecord $sourcePath
$candidateAfter = Get-PubFileRecord $candidatePath
if ($sourceAfter.sha256 -ne $sourceBefore.sha256) {
    throw "Source PUB changed during native lifecycle"
}
if ($candidateAfter.sha256 -ne $candidateBefore.sha256) {
    throw "Generated candidate changed during native lifecycle"
}

Push-Location $repoRoot
try {
    $verifyOutput = & cargo run --quiet --manifest-path vendor/producer-a/Cargo.toml -p pub-writer --bin pub_story_native_handoff -- verify --pub $savedPath --manifest $manifestPath
    if ($LASTEXITCODE -ne 0) {
        throw "Rar semantic verification failed after Publisher SaveAs"
    }
}
finally {
    Pop-Location
}
$semantic = ($verifyOutput -join [Environment]::NewLine) | ConvertFrom-Json
$savedRecord = Get-PubFileRecord $savedPath

$receipt = [ordered]@{
    schema = "chaptera.pub-native-story-roundtrip-receipt.v1"
    handoff = [ordered]@{
        source_sha256 = [string]$manifest.source_sha256
        candidate_sha256 = [string]$manifest.candidate_sha256
        story_syid = [int]$manifest.story_syid
        expected_story_text_sha256 = [string]$manifest.after_text_sha256
    }
    publisher = [ordered]@{
        version = [string]$publisher.version.value
        build = [string]$publisher.build.value
        name = if ($publisher.name.state -eq "value") { [string]$publisher.name.value } else { $null }
    }
    lifecycle = [ordered]@{
        candidate_open = $true
        save_as = $true
        fresh_reopen = $true
        candidate_open_snapshot = $openSnapshot
        fresh_reopen_snapshot = $reopenSnapshot
    }
    preservation = [ordered]@{
        source_pub_unchanged = ($sourceAfter.sha256 -eq $sourceBefore.sha256)
        candidate_pub_unchanged = ($candidateAfter.sha256 -eq $candidateBefore.sha256)
        native_saved_sha256 = $savedRecord.sha256
    }
    semantic = [ordered]@{
        verified_by_current_rar = ($semantic.status -eq "valid")
        story_syid = [int]$semantic.story_syid
        story_text_sha256 = [string]$semantic.story_text_sha256
        story_utf16_len = [int]$semantic.story_utf16_len
    }
}
$receipt | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $receiptPath -Encoding utf8

Write-Host "PUB native Story roundtrip PASS"
Write-Host "Receipt: $receiptPath"
Write-Host "Native saved PUB retained locally: $savedPath"
