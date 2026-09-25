param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedExperiment = "GROUP-HISTORY-01"
$ExpectedFixtureSha256 = "5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b"
$ExpectedPublisherExeSha256 = "e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b"
$PbFilePublication = 1
$MsoShapeRectangle = 1
$ShapeNames = [object[]]@("GH_A", "GH_B", "GH_C")

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
$privateDir = Join-Path $OutputRoot "private\group-history-01"
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
    try { $Document.Saved = $true } catch {}
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

function Add-OracleTag {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][string]$Value
    )
    $Shape.Tags.Add("PUB_ORACLE_ID", $Value) | Out-Null
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

function Get-ShapeRecord {
    param([Parameter(Mandatory = $true)]$Shape)

    $children = @()
    try {
        for ($i = 1; $i -le [int]$Shape.GroupItems.Count; $i++) {
            $child = $Shape.GroupItems.Item($i)
            $children += [ordered]@{
                index = $i
                name = Get-PubSafeValue { [string]$child.Name } "GroupItem.Name"
                shape_id = Get-PubSafeValue { [int]$child.ID } "GroupItem.ID"
                oracle_tag = Get-OracleTagValue -Shape $child
                z_order = Get-PubSafeValue { [int]$child.ZOrderPosition } "GroupItem.ZOrderPosition"
            }
        }
    }
    catch {
        $children = @()
    }

    return [ordered]@{
        name = Get-PubSafeValue { [string]$Shape.Name } "Shape.Name"
        shape_id = Get-PubSafeValue { [int]$Shape.ID } "Shape.ID"
        shape_type = Get-PubSafeValue { [int]$Shape.Type } "Shape.Type"
        oracle_tag = Get-OracleTagValue -Shape $Shape
        z_order = Get-PubSafeValue { [int]$Shape.ZOrderPosition } "Shape.ZOrderPosition"
        left = Get-PubSafeValue { [double]$Shape.Left } "Shape.Left"
        top = Get-PubSafeValue { [double]$Shape.Top } "Shape.Top"
        width = Get-PubSafeValue { [double]$Shape.Width } "Shape.Width"
        height = Get-PubSafeValue { [double]$Shape.Height } "Shape.Height"
        group_item_count = Get-PubSafeValue { [int]$Shape.GroupItems.Count } "Shape.GroupItems.Count"
        group_items = $children
    }
}

function Get-DocumentSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    if ([int]$Document.Pages.Count -ne 1) {
        throw "$Phase expected one page; found $([int]$Document.Pages.Count)"
    }

    $page = $Document.Pages.Item(1)
    $shapes = @()
    for ($i = 1; $i -le [int]$page.Shapes.Count; $i++) {
        $shapes += Get-ShapeRecord -Shape $page.Shapes.Item($i)
    }

    return [ordered]@{
        phase = $Phase
        document_saved = Get-PubSafeValue { [bool]$Document.Saved } "Document.Saved"
        page_shape_count = [int]$page.Shapes.Count
        scratch_shape_count = Get-PubSafeValue { [int]$Document.ScratchArea.Shapes.Count } "ScratchArea.Shapes.Count"
        shapes = $shapes
    }
}

function Add-BaseShapes {
    param([Parameter(Mandatory = $true)]$Document)

    if ([int]$Document.Pages.Count -ne 1) {
        throw "Blank fixture must have one page"
    }
    $page = $Document.Pages.Item(1)
    if ([int]$page.Shapes.Count -ne 0) {
        throw "Blank fixture must have zero page shapes"
    }

    $specs = @(
        [ordered]@{ name = "GH_A"; tag = "GROUP_HISTORY_A"; left = 72;  top = 72; width = 90; height = 60 },
        [ordered]@{ name = "GH_B"; tag = "GROUP_HISTORY_B"; left = 198; top = 72; width = 90; height = 60 },
        [ordered]@{ name = "GH_C"; tag = "GROUP_HISTORY_C"; left = 324; top = 72; width = 90; height = 60 }
    )

    foreach ($spec in $specs) {
        $shape = $page.Shapes.AddShape(
            $MsoShapeRectangle,
            [double]$spec.left,
            [double]$spec.top,
            [double]$spec.width,
            [double]$spec.height
        )
        $shape.Name = [string]$spec.name
        Add-OracleTag -Shape $shape -Value ([string]$spec.tag)
    }

    return $page
}

function Group-ABC {
    param([Parameter(Mandatory = $true)]$Page)
    $range = $Page.Shapes.Range($ShapeNames)
    $group = $range.Group()
    $group.Name = "GH_G"
    Add-OracleTag -Shape $group -Value "GROUP_HISTORY_G"
    return $group
}

function Get-CurrentABCNames {
    param([Parameter(Mandatory = $true)]$Page)
    $names = @()
    foreach ($expected in @("GH_A", "GH_B", "GH_C")) {
        $found = $false
        for ($i = 1; $i -le [int]$Page.Shapes.Count; $i++) {
            $shape = $Page.Shapes.Item($i)
            if ([string]$shape.Name -eq $expected) {
                $names += $expected
                $found = $true
                break
            }
        }
        if (-not $found) {
            throw "Expected ungrouped member name '$expected' is missing"
        }
    }
    return [object[]]$names
}

function Try-Regroup {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $result = [ordered]@{
        phase = $Phase
        state = "not_attempted"
        error = $null
        before = Get-DocumentSnapshot -Document $Document -Phase "$Phase-before"
        returned_group = $null
        after = $null
    }

    try {
        $page = $Document.Pages.Item(1)
        $names = Get-CurrentABCNames -Page $page
        $range = $page.Shapes.Range($names)
        $group = $range.Regroup()
        $result.state = "success"
        $result.returned_group = Get-ShapeRecord -Shape $group
        $result.after = Get-DocumentSnapshot -Document $Document -Phase "$Phase-after"
    }
    catch {
        $result.state = "error"
        $result.error = Get-ExceptionRecord $_.Exception
        try {
            $result.after = Get-DocumentSnapshot -Document $Document -Phase "$Phase-after-error"
        }
        catch {}
    }

    return $result
}

function Initialize-Arm {
    param([Parameter(Mandatory = $true)][string]$ArmId)

    $armRoot = Join-Path $privateDir $ArmId
    New-Item -ItemType Directory -Force -Path $armRoot | Out-Null
    $inputPath = Join-Path $armRoot "input.pub"
    Copy-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE -Destination $inputPath

    return [ordered]@{
        arm_root = $armRoot
        input_path = $inputPath
        input_receipt = Get-FileReceipt $inputPath
    }
}

function Open-ArmDocument {
    param([Parameter(Mandatory = $true)][string]$Path)
    $application = New-PubPublisherApplication
    try {
        $document = $application.Open($Path, $false, $false)
        return [ordered]@{
            application = $application
            document = $document
        }
    }
    catch {
        Close-PubPublisherApplication $application
        throw
    }
}

function Save-ArmDocument {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $Document.SaveAs($Path, $PbFilePublication, $false)
    return Get-FileReceipt $Path
}

function Close-ArmPair {
    param($Pair)
    if ($null -eq $Pair) { return }
    Close-ComDocument $Pair.document
    Close-PubPublisherApplication $Pair.application
}

function Run-S0 {
    $ctx = Initialize-Arm -ArmId "S0-independent"
    $pair = $null
    $outputPath = Join-Path $ctx.arm_root "independent.pub"
    $result = [ordered]@{
        arm = "S0"
        description = "independent A/B/C save control; never grouped"
        input = $ctx.input_receipt
        before_save = $null
        saved = $null
        reopen = $null
        control_regroup = $null
    }
    try {
        $pair = Open-ArmDocument -Path $ctx.input_path
        Add-BaseShapes -Document $pair.document | Out-Null
        $result.before_save = Get-DocumentSnapshot -Document $pair.document -Phase "S0-before-save"
        $result.saved = Save-ArmDocument -Document $pair.document -Path $outputPath
    }
    finally {
        Close-ArmPair $pair
    }

    $pair = $null
    try {
        $pair = Open-ArmDocument -Path $outputPath
        $result.reopen = Get-DocumentSnapshot -Document $pair.document -Phase "S0-reopen"
        $result.control_regroup = Try-Regroup -Document $pair.document -Phase "S0-independent-regroup-control"
    }
    finally {
        Close-ArmPair $pair
    }
    return $result
}

function Run-S1 {
    $ctx = Initialize-Arm -ArmId "S1-grouped"
    $pair = $null
    $outputPath = Join-Path $ctx.arm_root "grouped.pub"
    $result = [ordered]@{
        arm = "S1"
        description = "Group(A,B,C)=G then Save/reopen"
        before_group = $null
        after_group = $null
        old_group = $null
        saved = $null
        reopen = $null
    }
    try {
        $pair = Open-ArmDocument -Path $ctx.input_path
        $page = Add-BaseShapes -Document $pair.document
        $result.before_group = Get-DocumentSnapshot -Document $pair.document -Phase "S1-before-group"
        $group = Group-ABC -Page $page
        $result.old_group = Get-ShapeRecord -Shape $group
        $result.after_group = Get-DocumentSnapshot -Document $pair.document -Phase "S1-after-group"
        $result.saved = Save-ArmDocument -Document $pair.document -Path $outputPath
    }
    finally {
        Close-ArmPair $pair
    }

    $pair = $null
    try {
        $pair = Open-ArmDocument -Path $outputPath
        $result.reopen = Get-DocumentSnapshot -Document $pair.document -Phase "S1-reopen"
    }
    finally {
        Close-ArmPair $pair
    }
    return $result
}

function Run-S2 {
    $ctx = Initialize-Arm -ArmId "S2-inmemory"
    $pair = $null
    $result = [ordered]@{
        arm = "S2"
        description = "Group→Ungroup→Regroup in-memory without Save"
        grouped = $null
        ungrouped = $null
        regroup = $null
    }
    try {
        $pair = Open-ArmDocument -Path $ctx.input_path
        $page = Add-BaseShapes -Document $pair.document
        $group = Group-ABC -Page $page
        $result.grouped = [ordered]@{
            group = Get-ShapeRecord -Shape $group
            snapshot = Get-DocumentSnapshot -Document $pair.document -Phase "S2-grouped"
        }
        $ungroupedRange = $group.Ungroup()
        $result.ungrouped = Get-DocumentSnapshot -Document $pair.document -Phase "S2-ungrouped"
        $result.regroup = Try-Regroup -Document $pair.document -Phase "S2-inmemory-regroup"
    }
    finally {
        Close-ArmPair $pair
    }
    return $result
}

function Run-S3 {
    $ctx = Initialize-Arm -ArmId "S3-save-same-session"
    $pair = $null
    $ungroupedPath = Join-Path $ctx.arm_root "ungrouped-saved.pub"
    $result = [ordered]@{
        arm = "S3"
        description = "Group→Ungroup→Save→Regroup in same session"
        ungrouped_before_save = $null
        saved_ungrouped = $null
        after_save = $null
        regroup = $null
    }
    try {
        $pair = Open-ArmDocument -Path $ctx.input_path
        $page = Add-BaseShapes -Document $pair.document
        $group = Group-ABC -Page $page
        [void]$group.Ungroup()
        $result.ungrouped_before_save = Get-DocumentSnapshot -Document $pair.document -Phase "S3-ungrouped-before-save"
        $result.saved_ungrouped = Save-ArmDocument -Document $pair.document -Path $ungroupedPath
        $result.after_save = Get-DocumentSnapshot -Document $pair.document -Phase "S3-after-save"
        $result.regroup = Try-Regroup -Document $pair.document -Phase "S3-regroup-after-save-same-session"
    }
    finally {
        Close-ArmPair $pair
    }
    return $result
}

function New-SavedUngroupedArm {
    param([Parameter(Mandatory = $true)][string]$ArmId)

    $ctx = Initialize-Arm -ArmId $ArmId
    $pair = $null
    $ungroupedPath = Join-Path $ctx.arm_root "ungrouped-saved.pub"
    $beforeSave = $null
    $afterSave = $null
    $receipt = $null
    try {
        $pair = Open-ArmDocument -Path $ctx.input_path
        $page = Add-BaseShapes -Document $pair.document
        $group = Group-ABC -Page $page
        [void]$group.Ungroup()
        $beforeSave = Get-DocumentSnapshot -Document $pair.document -Phase "$ArmId-ungrouped-before-save"
        $receipt = Save-ArmDocument -Document $pair.document -Path $ungroupedPath
        $afterSave = Get-DocumentSnapshot -Document $pair.document -Phase "$ArmId-ungrouped-after-save"
    }
    finally {
        Close-ArmPair $pair
    }

    return [ordered]@{
        path = $ungroupedPath
        receipt = $receipt
        before_save = $beforeSave
        after_save = $afterSave
    }
}

function Run-S4 {
    $saved = New-SavedUngroupedArm -ArmId "S4-one-reopen"
    $pair = $null
    $result = [ordered]@{
        arm = "S4"
        description = "Group→Ungroup→Save→close→reopen→Regroup"
        saved_ungrouped = $saved.receipt
        before_save = $saved.before_save
        after_save = $saved.after_save
        reopen = $null
        regroup = $null
    }
    try {
        $pair = Open-ArmDocument -Path $saved.path
        $result.reopen = Get-DocumentSnapshot -Document $pair.document -Phase "S4-reopen"
        $result.regroup = Try-Regroup -Document $pair.document -Phase "S4-regroup-after-one-reopen"
    }
    finally {
        Close-ArmPair $pair
    }
    return $result
}

function Run-S5 {
    $saved = New-SavedUngroupedArm -ArmId "S5-two-reopens"
    $pair = $null
    $firstReopen = $null
    try {
        $pair = Open-ArmDocument -Path $saved.path
        $firstReopen = Get-DocumentSnapshot -Document $pair.document -Phase "S5-first-reopen-no-mutation"
    }
    finally {
        Close-ArmPair $pair
    }

    $pair = $null
    $result = [ordered]@{
        arm = "S5"
        description = "Group→Ungroup→Save→close/reopen twice→Regroup"
        saved_ungrouped = $saved.receipt
        before_save = $saved.before_save
        after_save = $saved.after_save
        first_reopen = $firstReopen
        second_reopen = $null
        regroup = $null
    }
    try {
        $pair = Open-ArmDocument -Path $saved.path
        $result.second_reopen = Get-DocumentSnapshot -Document $pair.document -Phase "S5-second-reopen"
        $result.regroup = Try-Regroup -Document $pair.document -Phase "S5-regroup-after-two-reopens"
    }
    finally {
        Close-ArmPair $pair
    }
    return $result
}

$results = @()
$results += Run-S0
$results += Run-S1
$results += Run-S2
$results += Run-S3
$results += Run-S4
$results += Run-S5

$s0 = $results | Where-Object { $_.arm -eq "S0" } | Select-Object -First 1
$s2 = $results | Where-Object { $_.arm -eq "S2" } | Select-Object -First 1
$s3 = $results | Where-Object { $_.arm -eq "S3" } | Select-Object -First 1
$s4 = $results | Where-Object { $_.arm -eq "S4" } | Select-Object -First 1
$s5 = $results | Where-Object { $_.arm -eq "S5" } | Select-Object -First 1

$classification = [ordered]@{
    independent_control_regroup_state = $s0.control_regroup.state
    in_memory_regroup_state = $s2.regroup.state
    after_save_same_session_regroup_state = $s3.regroup.state
    after_one_reopen_regroup_state = $s4.regroup.state
    after_two_reopens_regroup_state = $s5.regroup.state
    history_scope = "unresolved"
}
if ($s0.control_regroup.state -eq "success") {
    $classification.history_scope = "invalid-control-regroup-succeeded"
}
elseif ($s4.regroup.state -eq "success" -or $s5.regroup.state -eq "success") {
    $classification.history_scope = "persisted-across-reopen-candidate"
}
elseif ($s2.regroup.state -eq "success" -and $s3.regroup.state -eq "success" -and
        $s4.regroup.state -eq "error" -and $s5.regroup.state -eq "error") {
    $classification.history_scope = "session-runtime-candidate"
}

$summary = [ordered]@{
    schema = "pub-group-history-01/summary/v1"
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
        raw_history_carrier_claimed = $false
        statement = "This first arm classifies COM-visible previous-group lifetime only. If reopen Regroup succeeds, the private saved ungrouped PUB must be structurally diffed against S0 before naming any persisted carrier."
    }
    arms = $results
    classification = $classification
}

Write-PubJson -Value $summary -Path (Join-Path $analysisDir "group-history-01.json")

$logLines = @(
    "experiment=$ExpectedExperiment",
    "fixture_sha256=$fixtureHash",
    "publisher_exe_sha256=$publisherHash",
    "independent_control_regroup=$($classification.independent_control_regroup_state)",
    "in_memory_regroup=$($classification.in_memory_regroup_state)",
    "after_save_same_session_regroup=$($classification.after_save_same_session_regroup_state)",
    "after_one_reopen_regroup=$($classification.after_one_reopen_regroup_state)",
    "after_two_reopens_regroup=$($classification.after_two_reopens_regroup_state)",
    "history_scope=$($classification.history_scope)"
)
$logLines | Set-Content -LiteralPath (Join-Path $logDir "group-history-01.txt") -Encoding ASCII
