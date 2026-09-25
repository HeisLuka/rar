param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedExperiment = "PICSTATE-01"
$ExpectedFixtureSha256 = "5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b"
$ExpectedPublisherExeSha256 = "e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b"
$ExpectedSentinelSha256 = "54ac15d54e246a28e6a77173345b84360106257a0a6dacd8db0cfdfe59a31b9b"
$SentinelBase64 = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAGUlEQVR42mMQMgl7WKWDSTJgFQWSDINSBwDF7kRhaek/1wAAAABJRU5ErkJggg=="
$PbFilePublication = 1
$MsoFalse = 0
$MsoTrue = -1

$packet = Get-Content -LiteralPath $PacketPath -Raw | ConvertFrom-Json
if ([string]$packet.id -ne $ExpectedExperiment) {
    throw "Unexpected experiment id: $($packet.id)"
}
if ([string]::IsNullOrWhiteSpace([string]$env:PUB_RESEARCH_FIXTURE)) {
    throw "PUB_RESEARCH_FIXTURE was not resolved by prepare_native_run.ps1."
}
if ([string]::IsNullOrWhiteSpace([string]$env:PUB_RESEARCH_PUBLISHER_EXE)) {
    throw "PUB_RESEARCH_PUBLISHER_EXE was not resolved by prepare_native_run.ps1."
}

$fixtureHash = (Get-FileHash -LiteralPath $env:PUB_RESEARCH_FIXTURE -Algorithm SHA256).Hash.ToLowerInvariant()
if ($fixtureHash -ne $ExpectedFixtureSha256) {
    throw "Exact blank fixture mismatch: $fixtureHash"
}
$publisherHash = (Get-FileHash -LiteralPath $env:PUB_RESEARCH_PUBLISHER_EXE -Algorithm SHA256).Hash.ToLowerInvariant()
if ($publisherHash -ne $ExpectedPublisherExeSha256) {
    throw "Publisher executable SHA-256 mismatch: $publisherHash"
}

Import-Module (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "windows\pub-runtime\PubRuntime.psm1") -Force

$analysisDir = Join-Path $OutputRoot "analysis"
$logDir = Join-Path $OutputRoot "logs"
$privateDir = Join-Path $OutputRoot "private\picstate-01"
New-Item -ItemType Directory -Force -Path $analysisDir | Out-Null
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
New-Item -ItemType Directory -Force -Path $privateDir | Out-Null

function Get-ExceptionRecord {
    param([Parameter(Mandatory = $true)]$Exception)
    return [ordered]@{
        hresult = if ($null -ne $Exception.HResult) { Format-PubHResult ([int]$Exception.HResult) } else { $null }
        message = [string]$Exception.Message
    }
}

function Close-ComDocument {
    param($Document)
    if ($null -eq $Document) { return }
    try { $Document.Close() } catch {}
    try { [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Document) } catch {}
}

function Get-FileReceipt {
    param([Parameter(Mandatory = $true)][string]$Path)
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        name = [string]$item.Name
        size = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Get-DirectorySnapshot {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return @() }
    $rows = @()
    foreach ($item in @(Get-ChildItem -LiteralPath $Path -File | Sort-Object Name)) {
        $rows += [ordered]@{
            name = [string]$item.Name
            extension = [string]$item.Extension
            size = [int64]$item.Length
            sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    return $rows
}

function Protect-PrivateText {
    param($Value)
    if ($null -eq $Value) { return $null }
    $text = [string]$Value
    if ([string]::IsNullOrEmpty($text)) { return $text }
    $privateFull = [System.IO.Path]::GetFullPath($privateDir).TrimEnd('\')
    return $text.Replace($privateFull, "<PRIVATE_ROOT>")
}

function Get-OracleTagValue {
    param([Parameter(Mandatory = $true)]$Shape)
    try {
        for ($i = 1; $i -le [int]$Shape.Tags.Count; $i++) {
            $tag = $Shape.Tags.Item($i)
            if ([string]$tag.Name -eq "PUB_ORACLE_ID") {
                return [string]$tag.Value
            }
        }
    }
    catch {
        return $null
    }
    return $null
}

function Find-TaggedShape {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][string]$TagValue
    )
    $matches = @()
    for ($pageIndex = 1; $pageIndex -le [int]$Document.Pages.Count; $pageIndex++) {
        $page = $Document.Pages.Item($pageIndex)
        for ($shapeIndex = 1; $shapeIndex -le [int]$page.Shapes.Count; $shapeIndex++) {
            $shape = $page.Shapes.Item($shapeIndex)
            if ((Get-OracleTagValue -Shape $shape) -eq $TagValue) {
                $matches += $shape
            }
        }
    }
    if ($matches.Count -ne 1) {
        throw "Expected exactly one PUB_ORACLE_ID=$TagValue; found $($matches.Count)"
    }
    return $matches[0]
}

function Get-PictureSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $sourceFullName = Get-PubSafeValue { [string]$Shape.LinkFormat.SourceFullName } "Shape.LinkFormat.SourceFullName"
    if ($sourceFullName.state -eq "value") {
        $sourceFullName.value = Protect-PrivateText $sourceFullName.value
    }

    $fileName = Get-PubSafeValue { [string]$Shape.PictureFormat.FileName } "Shape.PictureFormat.FileName"
    if ($fileName.state -eq "value") {
        $fileName.value = Protect-PrivateText $fileName.value
    }

    return [ordered]@{
        phase = $Phase
        oracle_tag = Get-OracleTagValue -Shape $Shape
        shape_id = Get-PubSafeValue { [int]$Shape.ID } "Shape.ID"
        shape_type = Get-PubSafeValue { [int]$Shape.Type } "Shape.Type"
        left = Get-PubSafeValue { [double]$Shape.Left } "Shape.Left"
        top = Get-PubSafeValue { [double]$Shape.Top } "Shape.Top"
        width = Get-PubSafeValue { [double]$Shape.Width } "Shape.Width"
        height = Get-PubSafeValue { [double]$Shape.Height } "Shape.Height"
        picture = [ordered]@{
            is_linked = Get-PubSafeValue { [bool]$Shape.PictureFormat.IsLinked } "PictureFormat.IsLinked"
            file_name = $fileName
            file_size = Get-PubSafeValue { [int64]$Shape.PictureFormat.FileSize } "PictureFormat.FileSize"
            original_file_size = Get-PubSafeValue { [int64]$Shape.PictureFormat.OriginalFileSize } "PictureFormat.OriginalFileSize"
            linked_file_status = Get-PubSafeValue { [int]$Shape.PictureFormat.LinkedFileStatus } "PictureFormat.LinkedFileStatus"
            image_format = Get-PubSafeValue { [int]$Shape.PictureFormat.ImageFormat } "PictureFormat.ImageFormat"
            source_full_name = $sourceFullName
        }
    }
}

function New-SentinelPng {
    param([Parameter(Mandatory = $true)][string]$Path)
    [System.IO.File]::WriteAllBytes($Path, [Convert]::FromBase64String($SentinelBase64))
    $receipt = Get-FileReceipt $Path
    if ($receipt.sha256 -ne $ExpectedSentinelSha256) {
        throw "Generated sentinel SHA-256 mismatch: $($receipt.sha256)"
    }
    return $receipt
}

function Invoke-PictureCell {
    param(
        [Parameter(Mandatory = $true)][string]$CellId,
        [Parameter(Mandatory = $true)][bool]$LinkToFile,
        [Parameter(Mandatory = $true)][bool]$SaveWithDocument
    )

    $armRoot = Join-Path $privateDir $CellId
    $sourceDir = Join-Path $armRoot "source"
    $publicationDir = Join-Path $armRoot "publication"
    New-Item -ItemType Directory -Force -Path $sourceDir | Out-Null
    New-Item -ItemType Directory -Force -Path $publicationDir | Out-Null

    $sentinelPath = Join-Path $sourceDir "PICSTATE-SENTINEL.png"
    $inputPath = Join-Path $publicationDir "input.pub"
    $outputPath = Join-Path $publicationDir "output.pub"
    $sentinelReceipt = New-SentinelPng -Path $sentinelPath
    Copy-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE -Destination $inputPath

    $tagValue = "PICSTATE_$CellId"
    $cell = [ordered]@{
        cell = $CellId
        requested = [ordered]@{
            link_to_file = $LinkToFile
            save_with_document = $SaveWithDocument
        }
        sentinel = $sentinelReceipt
        input_pub = Get-FileReceipt $inputPath
        publication_directory = [ordered]@{
            before = Get-DirectorySnapshot $publicationDir
            after_add_picture = $null
            after_save = $null
            after_reopen = $null
        }
        source_directory = [ordered]@{
            before = Get-DirectorySnapshot $sourceDir
            after = $null
        }
        add_picture = [ordered]@{
            state = "not_attempted"
            error = $null
        }
        save = [ordered]@{
            state = "not_attempted"
            output_pub = $null
            error = $null
        }
        reopen = [ordered]@{
            state = "not_attempted"
            snapshot = $null
            error = $null
        }
        before_save = $null
        after_save = $null
        classification = [ordered]@{
            create_succeeded = $false
            save_succeeded = $false
            reopen_succeeded = $false
            reopen_is_linked = $null
            shape_id_stable = $null
            publication_local_sentinel_copy_count = $null
        }
    }

    $application = $null
    $document = $null
    try {
        $application = New-PubPublisherApplication
        $document = $application.Open($inputPath, $false, $false)
        if ([int]$document.Pages.Count -ne 1) {
            throw "Expected one-page blank fixture; found $([int]$document.Pages.Count)"
        }

        $page = $document.Pages.Item(1)
        if ([int]$page.Shapes.Count -ne 0) {
            throw "Expected zero Page.Shapes in blank fixture; found $([int]$page.Shapes.Count)"
        }

        try {
            $linkFlag = if ($LinkToFile) { $MsoTrue } else { $MsoFalse }
            $saveFlag = if ($SaveWithDocument) { $MsoTrue } else { $MsoFalse }
            $shape = $page.Shapes.AddPicture(
                $sentinelPath,
                $linkFlag,
                $saveFlag,
                72,
                72,
                144,
                96
            )
            $shape.Tags.Add("PUB_ORACLE_ID", $tagValue) | Out-Null
            $cell.add_picture.state = "created"
            $cell.classification.create_succeeded = $true
            $cell.before_save = Get-PictureSnapshot -Shape $shape -Phase "after_add_picture_before_save"
            $cell.publication_directory.after_add_picture = Get-DirectorySnapshot $publicationDir
        }
        catch {
            $cell.add_picture.state = "error"
            $cell.add_picture.error = Get-ExceptionRecord $_.Exception
            $cell.publication_directory.after_add_picture = Get-DirectorySnapshot $publicationDir
        }

        if ($cell.classification.create_succeeded) {
            try {
                $document.SaveAs($outputPath, $PbFilePublication, $false)
                $cell.save.state = "saved"
                $cell.classification.save_succeeded = $true
                $cell.save.output_pub = Get-FileReceipt $outputPath
                $savedShape = Find-TaggedShape -Document $document -TagValue $tagValue
                $cell.after_save = Get-PictureSnapshot -Shape $savedShape -Phase "after_save"
            }
            catch {
                $cell.save.state = "error"
                $cell.save.error = Get-ExceptionRecord $_.Exception
            }
        }

        $cell.publication_directory.after_save = Get-DirectorySnapshot $publicationDir
        $cell.source_directory.after = Get-DirectorySnapshot $sourceDir
    }
    catch {
        if ($cell.add_picture.state -eq "not_attempted") {
            $cell.add_picture.state = "setup_error"
            $cell.add_picture.error = Get-ExceptionRecord $_.Exception
        }
    }
    finally {
        Close-ComDocument $document
        Close-PubPublisherApplication $application
    }

    if ($cell.classification.save_succeeded -and (Test-Path -LiteralPath $outputPath)) {
        $reopenApplication = $null
        $reopenDocument = $null
        try {
            $reopenApplication = New-PubPublisherApplication
            $reopenDocument = $reopenApplication.Open($outputPath, $true, $false)
            $reopenShape = Find-TaggedShape -Document $reopenDocument -TagValue $tagValue
            $cell.reopen.state = "opened"
            $cell.reopen.snapshot = Get-PictureSnapshot -Shape $reopenShape -Phase "fresh_reopen"
            $cell.classification.reopen_succeeded = $true

            $isLinked = $cell.reopen.snapshot.picture.is_linked
            if ($isLinked.state -eq "value") {
                $cell.classification.reopen_is_linked = [bool]$isLinked.value
            }

            $beforeId = $cell.before_save.shape_id
            $reopenId = $cell.reopen.snapshot.shape_id
            if ($beforeId.state -eq "value" -and $reopenId.state -eq "value") {
                $cell.classification.shape_id_stable = ([int]$beforeId.value -eq [int]$reopenId.value)
            }
        }
        catch {
            $cell.reopen.state = "error"
            $cell.reopen.error = Get-ExceptionRecord $_.Exception
        }
        finally {
            Close-ComDocument $reopenDocument
            Close-PubPublisherApplication $reopenApplication
        }
    }

    $cell.publication_directory.after_reopen = Get-DirectorySnapshot $publicationDir

    $copyCount = 0
    foreach ($row in @($cell.publication_directory.after_reopen)) {
        if ([string]$row.sha256 -eq $ExpectedSentinelSha256) {
            $copyCount++
        }
    }
    $cell.classification.publication_local_sentinel_copy_count = $copyCount

    return $cell
}

$cells = @(
    [ordered]@{ id = "FF"; link = $false; save = $false },
    [ordered]@{ id = "FT"; link = $false; save = $true },
    [ordered]@{ id = "TF"; link = $true;  save = $false },
    [ordered]@{ id = "TT"; link = $true;  save = $true }
)

$results = @()
foreach ($spec in $cells) {
    $results += Invoke-PictureCell -CellId $spec.id -LinkToFile $spec.link -SaveWithDocument $spec.save
}

$summary = [ordered]@{
    schema = "pub-picstate-01/summary/v1"
    experiment_id = $ExpectedExperiment
    fixture = [ordered]@{
        sha256 = $fixtureHash
        size = [int64](Get-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE).Length
    }
    publisher = [ordered]@{
        exe_sha256 = $publisherHash
        expected_version_prefix = "16.0.12527."
    }
    sentinel = [ordered]@{
        format = "PNG"
        width = 8
        height = 8
        sha256 = $ExpectedSentinelSha256
        generated_from_embedded_bytes = $true
    }
    evidence_boundary = [ordered]@{
        native_pub_outputs_private = $true
        sentinel_bytes_publicly_reproducible = $true
        raw_persistence_semantics_claimed = $false
        statement = "This arm establishes the COM creation/reopen truth table and publication-directory side effects only. BLIP/BStore/EscherDelay/path carrier attribution requires a separate private-PUB structural join."
    }
    cells = $results
}

$summaryPath = Join-Path $analysisDir "picstate-01.json"
Write-PubJson -Value $summary -Path $summaryPath

$logLines = @(
    "experiment=$ExpectedExperiment",
    "fixture_sha256=$fixtureHash",
    "publisher_exe_sha256=$publisherHash",
    "sentinel_sha256=$ExpectedSentinelSha256"
)
foreach ($cell in $results) {
    $logLines += "cell=$($cell.cell) requested_link=$($cell.requested.link_to_file) requested_save=$($cell.requested.save_with_document) create=$($cell.add_picture.state) save=$($cell.save.state) reopen=$($cell.reopen.state) reopen_is_linked=$($cell.classification.reopen_is_linked) local_copy_count=$($cell.classification.publication_local_sentinel_copy_count)"
}
$logLines | Set-Content -LiteralPath (Join-Path $logDir "picstate-01.txt") -Encoding ASCII
