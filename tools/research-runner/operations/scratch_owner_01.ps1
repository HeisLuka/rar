param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedExperiment = "SCRATCH-OWNER-01"
$ExpectedFixtureSha256 = "5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b"
$ExpectedPublisherExeSha256 = "e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b"
$PbFilePublication = 1
$MsoShapeRectangle = 1
$PageTag = "SCRATCH_OWNER_PAGE"
$ScratchTag = "SCRATCH_OWNER_SCRATCH"

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
    throw "Exact generated-blank fixture mismatch: $fixtureHash"
}
$publisherHash = (Get-FileHash -LiteralPath $env:PUB_RESEARCH_PUBLISHER_EXE -Algorithm SHA256).Hash.ToLowerInvariant()
if ($publisherHash -ne $ExpectedPublisherExeSha256) {
    throw "Publisher executable SHA-256 mismatch: $publisherHash"
}

Import-Module (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "windows\pub-runtime\PubRuntime.psm1") -Force

$analysisDir = Join-Path $OutputRoot "analysis"
$logDir = Join-Path $OutputRoot "logs"
$privateDir = Join-Path $OutputRoot "private\scratch-owner-01"
New-Item -ItemType Directory -Force -Path $analysisDir | Out-Null
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
New-Item -ItemType Directory -Force -Path $privateDir | Out-Null

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

function Get-TagSnapshot {
    param([Parameter(Mandatory = $true)]$Shape)
    $rows = @()
    try {
        for ($i = 1; $i -le [int]$Shape.Tags.Count; $i++) {
            $tag = $Shape.Tags.Item($i)
            $rows += [ordered]@{
                name = [string]$tag.Name
                value = [string]$tag.Value
            }
        }
    }
    catch {
        $rows += [ordered]@{
            error = $_.Exception.Message
        }
    }
    return $rows
}

function Find-TaggedShape {
    param(
        [Parameter(Mandatory = $true)]$Shapes,
        [Parameter(Mandatory = $true)][string]$TagValue,
        [Parameter(Mandatory = $true)][string]$OwnerDomain
    )

    $matches = @()
    for ($i = 1; $i -le [int]$Shapes.Count; $i++) {
        $shape = $Shapes.Item($i)
        if ((Get-OracleTagValue -Shape $shape) -eq $TagValue) {
            $matches += [ordered]@{
                owner_domain = $OwnerDomain
                index = $i
                shape = $shape
            }
        }
    }
    if ($matches.Count -ne 1) {
        throw "Expected exactly one tag '$TagValue' in $OwnerDomain; found $($matches.Count)"
    }
    return $matches[0]
}

function Get-ShapeSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Target,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $shape = $Target.shape
    return [ordered]@{
        phase = $Phase
        owner_domain = [string]$Target.owner_domain
        owner_index = [int]$Target.index
        oracle_tag = Get-OracleTagValue -Shape $shape
        shape_id = Get-PubSafeValue { [int]$shape.ID } "Shape.ID"
        shape_name = Get-PubSafeValue { [string]$shape.Name } "Shape.Name"
        shape_type = Get-PubSafeValue { [int]$shape.Type } "Shape.Type"
        is_excess = Get-PubSafeValue { [bool]$shape.IsExcess } "Shape.IsExcess"
        left = Get-PubSafeValue { [double]$shape.Left } "Shape.Left"
        top = Get-PubSafeValue { [double]$shape.Top } "Shape.Top"
        width = Get-PubSafeValue { [double]$shape.Width } "Shape.Width"
        height = Get-PubSafeValue { [double]$shape.Height } "Shape.Height"
        rotation = Get-PubSafeValue { [double]$shape.Rotation } "Shape.Rotation"
        tags = Get-TagSnapshot -Shape $shape
    }
}

function Get-OwnerSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    if ([int]$Document.Pages.Count -ne 1) {
        throw "$Phase requires exactly one publication page; found $([int]$Document.Pages.Count)"
    }

    $page = $Document.Pages.Item(1)
    $scratch = $Document.ScratchArea
    $pageTarget = Find-TaggedShape -Shapes $page.Shapes -TagValue $PageTag -OwnerDomain "page"
    $scratchTarget = Find-TaggedShape -Shapes $scratch.Shapes -TagValue $ScratchTag -OwnerDomain "scratch"

    $crossPageScratch = 0
    $crossScratchPage = 0
    for ($i = 1; $i -le [int]$page.Shapes.Count; $i++) {
        if ((Get-OracleTagValue -Shape $page.Shapes.Item($i)) -eq $ScratchTag) { $crossPageScratch++ }
    }
    for ($i = 1; $i -le [int]$scratch.Shapes.Count; $i++) {
        if ((Get-OracleTagValue -Shape $scratch.Shapes.Item($i)) -eq $PageTag) { $crossScratchPage++ }
    }

    return [ordered]@{
        phase = $Phase
        page_count = [int]$Document.Pages.Count
        page_shapes_count = [int]$page.Shapes.Count
        scratch_shapes_count = [int]$scratch.Shapes.Count
        surplus_shapes_count = Get-PubSafeValue { [int]$Document.SurplusShapes.Count } "Document.SurplusShapes.Count"
        cross_owner_tag_counts = [ordered]@{
            scratch_tag_found_on_page = $crossPageScratch
            page_tag_found_on_scratch = $crossScratchPage
        }
        page_shape = Get-ShapeSnapshot -Target $pageTarget -Phase $Phase
        scratch_shape = Get-ShapeSnapshot -Target $scratchTarget -Phase $Phase
    }
}

function Get-ExceptionRecord {
    param([Parameter(Mandatory = $true)]$Exception)
    return [ordered]@{
        hresult = if ($Exception.HResult) { Format-PubHResult ([int]$Exception.HResult) } else { $null }
        message = [string]$Exception.Message
    }
}

function Close-ComDocument {
    param($Document)
    if ($null -eq $Document) { return }
    try { $Document.Close() } catch {}
    try { [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Document) } catch {}
}

$candidatePath = Join-Path $privateDir "scratch-owner-source.pub"
Copy-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE -Destination $candidatePath
$sourceRecord = Get-PubFileRecord $candidatePath

$summary = [ordered]@{
    schema = "pub-scratch-owner-01/summary/v1"
    experiment_id = $ExpectedExperiment
    fixture = [ordered]@{
        sha256 = $fixtureHash
        size = [int64](Get-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE).Length
    }
    publisher = [ordered]@{
        exe_sha256 = $publisherHash
        expected_version_prefix = "16.0.12527."
    }
    evidence_boundary = [ordered]@{
        native_pub_outputs_private = $true
        raw_owner_semantics_claimed = $false
        statement = "This arm proves only COM owner-domain persistence and identity observations. Contents/Escher/Oid/seqNum attribution requires a separate parser join."
    }
    mutation = [ordered]@{
        state = "not_attempted"
        error = $null
    }
    before_save = $null
    after_save = $null
    saved_file = $null
    reopen = [ordered]@{
        state = "not_attempted"
        snapshot = $null
        error = $null
    }
    classification = [ordered]@{
        owner_domains_persist = $null
        page_shape_id_stable = $null
        scratch_shape_id_stable = $null
    }
}

$application = $null
$document = $null
$savedPath = Join-Path $privateDir "scratch-owner-saved.pub"

try {
    $application = New-PubPublisherApplication
    $document = $application.Open($candidatePath, $false, $false)

    if ([int]$document.Pages.Count -ne 1) {
        throw "Blank fixture expected exactly one page; found $([int]$document.Pages.Count)"
    }

    $page = $document.Pages.Item(1)
    $scratch = $document.ScratchArea

    if ([int]$page.Shapes.Count -ne 0) {
        throw "Blank fixture expected zero Page.Shapes; found $([int]$page.Shapes.Count)"
    }
    if ([int]$scratch.Shapes.Count -ne 0) {
        throw "Blank fixture expected zero ScratchArea.Shapes; found $([int]$scratch.Shapes.Count)"
    }

    try {
        $pageShape = $page.Shapes.AddShape($MsoShapeRectangle, 72, 90, 180, 100)
        $pageShape.Tags.Add("PUB_ORACLE_ID", $PageTag) | Out-Null

        $scratchShape = $scratch.Shapes.AddShape($MsoShapeRectangle, 72, 90, 180, 100)
        $scratchShape.Tags.Add("PUB_ORACLE_ID", $ScratchTag) | Out-Null

        $summary.mutation.state = "ok"
    }
    catch {
        $summary.mutation.state = "error"
        $summary.mutation.error = Get-ExceptionRecord $_.Exception
        throw
    }

    $summary.before_save = Get-OwnerSnapshot -Document $document -Phase "before_save"

    $document.SaveAs($savedPath, $PbFilePublication, $false)
    $summary.saved_file = Get-PubFileRecord $savedPath
    $summary.after_save = Get-OwnerSnapshot -Document $document -Phase "after_save"
}
finally {
    Close-ComDocument $document
    Close-PubPublisherApplication $application
}

$reopenApplication = $null
$reopenDocument = $null
try {
    $reopenApplication = New-PubPublisherApplication
    $reopenDocument = $reopenApplication.Open($savedPath, $true, $false)
    $summary.reopen.state = "opened"
    $summary.reopen.snapshot = Get-OwnerSnapshot -Document $reopenDocument -Phase "reopen"

    $beforePageId = $summary.before_save.page_shape.shape_id
    $beforeScratchId = $summary.before_save.scratch_shape.shape_id
    $reopenPageId = $summary.reopen.snapshot.page_shape.shape_id
    $reopenScratchId = $summary.reopen.snapshot.scratch_shape.shape_id

    $summary.classification.owner_domains_persist = (
        [int]$summary.reopen.snapshot.cross_owner_tag_counts.scratch_tag_found_on_page -eq 0 -and
        [int]$summary.reopen.snapshot.cross_owner_tag_counts.page_tag_found_on_scratch -eq 0
    )

    if ($beforePageId.state -eq "value" -and $reopenPageId.state -eq "value") {
        $summary.classification.page_shape_id_stable = ([int]$beforePageId.value -eq [int]$reopenPageId.value)
    }
    if ($beforeScratchId.state -eq "value" -and $reopenScratchId.state -eq "value") {
        $summary.classification.scratch_shape_id_stable = ([int]$beforeScratchId.value -eq [int]$reopenScratchId.value)
    }
}
catch {
    $summary.reopen.state = "error"
    $summary.reopen.error = Get-ExceptionRecord $_.Exception
}
finally {
    Close-ComDocument $reopenDocument
    Close-PubPublisherApplication $reopenApplication
}

$summaryPath = Join-Path $analysisDir "scratch-owner-01.json"
Write-PubJson -Value $summary -Path $summaryPath

$logLines = @(
    "experiment=$ExpectedExperiment",
    "fixture_sha256=$fixtureHash",
    "publisher_exe_sha256=$publisherHash",
    "mutation_state=$($summary.mutation.state)",
    "reopen_state=$($summary.reopen.state)",
    "owner_domains_persist=$($summary.classification.owner_domains_persist)",
    "page_shape_id_stable=$($summary.classification.page_shape_id_stable)",
    "scratch_shape_id_stable=$($summary.classification.scratch_shape_id_stable)"
)
$logLines | Set-Content -LiteralPath (Join-Path $logDir "scratch-owner-01.txt") -Encoding ASCII
