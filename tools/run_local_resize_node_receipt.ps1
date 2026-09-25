param(
    [Parameter(Mandatory = $true)]
    [string]$Fixture,
    [Parameter(Mandatory = $true)]
    [string]$Output,
    [string]$ChapteraVersion = "0.1.0-local"
)

$ErrorActionPreference = "Stop"

if ($env:GITHUB_ACTIONS -eq "true") {
    throw "ResizeNode local-private receipt must not run inside GitHub Actions"
}

$fixturePath = (Resolve-Path $Fixture).Path
$outputPath = [System.IO.Path]::GetFullPath($Output)
$sourceHash = (Get-FileHash $fixturePath -Algorithm SHA256).Hash.ToLowerInvariant()
$documentId = [guid]::NewGuid().ToString()

cargo build -p chaptera-desktop --release --features resize-producer --bin chaptera-resize-producer
if ($LASTEXITCODE -ne 0) {
    throw "chaptera-resize-producer release build failed"
}

$producer = (Resolve-Path "target/release/chaptera-resize-producer.exe").Path
$binaryHash = (Get-FileHash $producer -Algorithm SHA256).Hash.ToLowerInvariant()

python tools/build_resize_node_producer_receipt.py `
    --source-hash $sourceHash `
    --document-id $documentId `
    --chaptera-version $ChapteraVersion `
    --platform windows `
    --binary-sha256 $binaryHash `
    --fixture-kind real_pub_sanitized `
    --projection-instance-admitted `
    --output $outputPath `
    -- $producer --fixture $fixturePath
if ($LASTEXITCODE -ne 0) {
    throw "ResizeNode producer receipt build failed"
}

python tools/validate_resize_node_producer_receipt.py $outputPath
if ($LASTEXITCODE -ne 0) {
    throw "ResizeNode producer receipt validation failed"
}

$receipt = Get-Content -Raw $outputPath | ConvertFrom-Json
if ($receipt.producer.integration -ne "local_private") {
    throw "ResizeNode receipt integration is not local_private"
}
if ($receipt.fixture_kind -ne "real_pub_sanitized") {
    throw "ResizeNode receipt fixture kind is not real_pub_sanitized"
}
if ($receipt.privacy.source_hash_in_receipt -ne $false -or
    $receipt.privacy.node_id_in_receipt -ne $false -or
    $receipt.privacy.local_path_in_receipt -ne $false) {
    throw "ResizeNode receipt privacy boundary widened"
}

Write-Host "ResizeNode local-private receipt validated: $outputPath"
