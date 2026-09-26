param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedExperiment = "ACTIVE-LIST-01"
$ExpectedFixtureSha256 = "5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b"
$ExpectedPublisherExeSha256 = "e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b"
$PbFilePublication = 1
$PbTextOrientationHorizontal = 1
$PbListTypeArabic = 0
$PbListTypeUppercaseRoman = 1
$PbListTypeArabicLeadingZero = 22
$PbListTypeBullet = 23

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
$privateDir = Join-Path $OutputRoot "private\active-list-01"
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

function Get-ParagraphSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $textRange = $Shape.TextFrame.TextRange
    $paragraph = $textRange.ParagraphFormat
    return [ordered]@{
        phase = $Phase
        oracle_tag = Get-OracleTagValue -Shape $Shape
        shape_id = Get-PubSafeValue { [int]$Shape.ID } "Shape.ID"
        shape_type = Get-PubSafeValue { [int]$Shape.Type } "Shape.Type"
        text = Get-PubSafeValue { [string]$textRange.Text } "TextRange.Text"
        list_type = Get-PubSafeValue { [int]$paragraph.ListType } "ParagraphFormat.ListType"
        list_number_separator = Get-PubSafeValue { [int]$paragraph.ListNumberSeparator } "ParagraphFormat.ListNumberSeparator"
        list_number_start = Get-PubSafeValue { [int]$paragraph.ListNumberStart } "ParagraphFormat.ListNumberStart"
        list_bullet_text = Get-PubSafeValue { [string]$paragraph.ListBulletText } "ParagraphFormat.ListBulletText"
        list_bullet_font_name = Get-PubSafeValue { [string]$paragraph.ListBulletFontName } "ParagraphFormat.ListBulletFontName"
        list_bullet_font_size = Get-PubSafeValue { [double]$paragraph.ListBulletFontSize } "ParagraphFormat.ListBulletFontSize"
    }
}

function Apply-ListMutation {
    param(
        [Parameter(Mandatory = $true)]$Paragraph,
        [Parameter(Mandatory = $true)]$Spec
    )

    switch ([string]$Spec.family) {
        "plain" {
            return
        }
        "list_type" {
            $Paragraph.SetListType([int]$Spec.value)
            return
        }
        "separator" {
            $Paragraph.SetListType($PbListTypeArabic)
            $Paragraph.ListNumberSeparator = [int]$Spec.value
            return
        }
        "number_start" {
            $Paragraph.SetListType($PbListTypeArabic)
            $Paragraph.ListNumberStart = [int]$Spec.value
            return
        }
        "bullet_text" {
            $Paragraph.SetListType($PbListTypeBullet, [string]$Spec.value)
            return
        }
        "bullet_default" {
            $Paragraph.SetListType($PbListTypeBullet)
            return
        }
        "bullet_font_name" {
            $Paragraph.SetListType($PbListTypeBullet, "*")
            $Paragraph.ListBulletFontName = [string]$Spec.value
            return
        }
        "bullet_font_size" {
            $Paragraph.SetListType($PbListTypeBullet, "*")
            $Paragraph.ListBulletFontSize = [double]$Spec.value
            return
        }
        default {
            throw "Unsupported mutation family: $($Spec.family)"
        }
    }
}

function Get-ReopenTargetValue {
    param(
        [Parameter(Mandatory = $true)]$Snapshot,
        [Parameter(Mandatory = $true)]$Spec
    )
    switch ([string]$Spec.family) {
        "list_type" { return $Snapshot.list_type }
        "separator" { return $Snapshot.list_number_separator }
        "number_start" { return $Snapshot.list_number_start }
        "bullet_text" { return $Snapshot.list_bullet_text }
        "bullet_font_name" { return $Snapshot.list_bullet_font_name }
        "bullet_font_size" { return $Snapshot.list_bullet_font_size }
        default { return $null }
    }
}

function Test-RequestedRoundTrip {
    param(
        $TargetRecord,
        [Parameter(Mandatory = $true)]$Spec
    )
    if ($null -eq $TargetRecord -or [string]$TargetRecord.state -ne "value") {
        return $null
    }

    switch ([string]$Spec.family) {
        "list_type" { return ([int]$TargetRecord.value -eq [int]$Spec.value) }
        "separator" { return ([int]$TargetRecord.value -eq [int]$Spec.value) }
        "number_start" { return ([int]$TargetRecord.value -eq [int]$Spec.value) }
        "bullet_text" { return ([string]$TargetRecord.value -eq [string]$Spec.value) }
        "bullet_font_name" { return ([string]$TargetRecord.value -ieq [string]$Spec.value) }
        "bullet_font_size" { return ([Math]::Abs([double]$TargetRecord.value - [double]$Spec.value) -lt 0.001) }
        default { return $null }
    }
}

function Invoke-ListArm {
    param([Parameter(Mandatory = $true)]$Spec)

    $armId = [string]$Spec.id
    $armRoot = Join-Path $privateDir $armId
    New-Item -ItemType Directory -Force -Path $armRoot | Out-Null
    $inputPath = Join-Path $armRoot "input.pub"
    $outputPath = Join-Path $armRoot "output.pub"
    Copy-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE -Destination $inputPath

    $tagValue = "ACTIVE_LIST_$armId"
    $arm = [ordered]@{
        arm = $armId
        requested = [ordered]@{
            family = [string]$Spec.family
            value = if ($Spec.Contains("value")) { $Spec.value } else { $null }
            semantic_name = if ($Spec.Contains("semantic_name")) { [string]$Spec.semantic_name } else { $null }
        }
        input_pub = Get-FileReceipt $inputPath
        mutation = [ordered]@{ state = "not_attempted"; error = $null }
        save = [ordered]@{ state = "not_attempted"; output_pub = $null; error = $null }
        reopen = [ordered]@{ state = "not_attempted"; snapshot = $null; error = $null }
        before_save = $null
        after_save = $null
        classification = [ordered]@{
            mutation_succeeded = $false
            save_succeeded = $false
            reopen_succeeded = $false
            shape_id_stable = $null
            requested_value_roundtrips = $null
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
            $shape = $page.Shapes.AddTextbox(
                $PbTextOrientationHorizontal,
                72,
                72,
                240,
                72
            )
            $shape.Tags.Add("PUB_ORACLE_ID", $tagValue) | Out-Null
            $shape.TextFrame.TextRange.Text = "List item"
            $paragraph = $shape.TextFrame.TextRange.ParagraphFormat
            Apply-ListMutation -Paragraph $paragraph -Spec $Spec
            $arm.mutation.state = "applied"
            $arm.classification.mutation_succeeded = $true
            $arm.before_save = Get-ParagraphSnapshot -Shape $shape -Phase "after_mutation_before_save"
        }
        catch {
            $arm.mutation.state = "error"
            $arm.mutation.error = Get-ExceptionRecord $_.Exception
        }

        if ($arm.classification.mutation_succeeded) {
            try {
                $document.SaveAs($outputPath, $PbFilePublication, $false)
                $arm.save.state = "saved"
                $arm.save.output_pub = Get-FileReceipt $outputPath
                $arm.classification.save_succeeded = $true
                $savedShape = Find-TaggedShape -Document $document -TagValue $tagValue
                $arm.after_save = Get-ParagraphSnapshot -Shape $savedShape -Phase "after_save"
            }
            catch {
                $arm.save.state = "error"
                $arm.save.error = Get-ExceptionRecord $_.Exception
            }
        }
    }
    catch {
        if ($arm.mutation.state -eq "not_attempted") {
            $arm.mutation.state = "setup_error"
            $arm.mutation.error = Get-ExceptionRecord $_.Exception
        }
    }
    finally {
        Close-ComDocument $document
        Close-PubPublisherApplication $application
    }

    if ($arm.classification.save_succeeded -and (Test-Path -LiteralPath $outputPath)) {
        $reopenApplication = $null
        $reopenDocument = $null
        try {
            $reopenApplication = New-PubPublisherApplication
            $reopenDocument = $reopenApplication.Open($outputPath, $true, $false)
            $reopenShape = Find-TaggedShape -Document $reopenDocument -TagValue $tagValue
            $arm.reopen.state = "opened"
            $arm.reopen.snapshot = Get-ParagraphSnapshot -Shape $reopenShape -Phase "fresh_reopen"
            $arm.classification.reopen_succeeded = $true

            $beforeId = $arm.before_save.shape_id
            $reopenId = $arm.reopen.snapshot.shape_id
            if ($beforeId.state -eq "value" -and $reopenId.state -eq "value") {
                $arm.classification.shape_id_stable = ([int]$beforeId.value -eq [int]$reopenId.value)
            }

            $target = Get-ReopenTargetValue -Snapshot $arm.reopen.snapshot -Spec $Spec
            $arm.classification.requested_value_roundtrips = Test-RequestedRoundTrip -TargetRecord $target -Spec $Spec
        }
        catch {
            $arm.reopen.state = "error"
            $arm.reopen.error = Get-ExceptionRecord $_.Exception
        }
        finally {
            Close-ComDocument $reopenDocument
            Close-PubPublisherApplication $reopenApplication
        }
    }

    return $arm
}

$arms = @(
    [ordered]@{ id = "C0"; family = "plain"; semantic_name = "plain_no_list_control" },
    [ordered]@{ id = "T0"; family = "list_type"; value = $PbListTypeArabic; semantic_name = "pbListTypeArabic" },
    [ordered]@{ id = "T1"; family = "list_type"; value = $PbListTypeUppercaseRoman; semantic_name = "pbListTypeUppercaseRoman" },
    [ordered]@{ id = "T22"; family = "list_type"; value = $PbListTypeArabicLeadingZero; semantic_name = "pbListTypeArabicLeadingZero" },
    [ordered]@{ id = "BD"; family = "bullet_default"; semantic_name = "pbListTypeBullet_default_dialog_bullet" },
    [ordered]@{ id = "BT_STAR"; family = "bullet_text"; value = "*"; semantic_name = "bullet_asterisk" },
    [ordered]@{ id = "BT_HASH"; family = "bullet_text"; value = "#"; semantic_name = "bullet_hash" },
    [ordered]@{ id = "SEP0"; family = "separator"; value = 0; semantic_name = "pbListSeparatorParenthesis" },
    [ordered]@{ id = "SEP1"; family = "separator"; value = 65536; semantic_name = "pbListSeparatorDoubleParen" },
    [ordered]@{ id = "SEP2"; family = "separator"; value = 131072; semantic_name = "pbListSeparatorPeriod" },
    [ordered]@{ id = "SEP3"; family = "separator"; value = 196608; semantic_name = "pbListSeparatorPlain" },
    [ordered]@{ id = "SEP4"; family = "separator"; value = 262144; semantic_name = "pbListSeparatorSquare" },
    [ordered]@{ id = "SEP5"; family = "separator"; value = 327680; semantic_name = "pbListSeparatorColon" },
    [ordered]@{ id = "SEP6"; family = "separator"; value = 393216; semantic_name = "pbListSeparatorDoubleSquare" },
    [ordered]@{ id = "SEP7"; family = "separator"; value = 458752; semantic_name = "pbListSeparatorDoubleHyphen" },
    [ordered]@{ id = "SEP8"; family = "separator"; value = 524288; semantic_name = "pbListSeparatorWideComma" },
    [ordered]@{ id = "N1"; family = "number_start"; value = 1; semantic_name = "list_number_start_1" },
    [ordered]@{ id = "N4"; family = "number_start"; value = 4; semantic_name = "list_number_start_4" },
    [ordered]@{ id = "N37"; family = "number_start"; value = 37; semantic_name = "list_number_start_37" },
    [ordered]@{ id = "FN_ARIAL"; family = "bullet_font_name"; value = "Arial"; semantic_name = "bullet_font_Arial" },
    [ordered]@{ id = "FN_VERDANA"; family = "bullet_font_name"; value = "Verdana"; semantic_name = "bullet_font_Verdana" },
    [ordered]@{ id = "FS12"; family = "bullet_font_size"; value = 12.0; semantic_name = "bullet_font_size_12" },
    [ordered]@{ id = "FS24"; family = "bullet_font_size"; value = 24.0; semantic_name = "bullet_font_size_24" }
)

$results = @()
foreach ($spec in $arms) {
    $results += Invoke-ListArm -Spec $spec
}

$summary = [ordered]@{
    schema = "pub-active-list-01/summary/v1"
    experiment_id = $ExpectedExperiment
    fixture = [ordered]@{
        sha256 = $fixtureHash
        size = [int64](Get-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE).Length
    }
    publisher = [ordered]@{
        exe_sha256 = $publisherHash
        expected_version_prefix = "16.0.12527."
    }
    public_api_vocabulary = [ordered]@{
        pbListTypeArabic = $PbListTypeArabic
        pbListTypeUppercaseRoman = $PbListTypeUppercaseRoman
        pbListTypeArabicLeadingZero = $PbListTypeArabicLeadingZero
        pbListTypeBullet = $PbListTypeBullet
        separator_values = @(0, 65536, 131072, 196608, 262144, 327680, 393216, 458752, 524288)
    }
    evidence_boundary = [ordered]@{
        native_pub_outputs_private = $true
        public_com_roundtrip_claimed = $true
        raw_opl_field_mapping_claimed = $false
        preset_bank_0x4a_immutability_claimed = $false
        statement = "This seam establishes controlled Publisher ParagraphFormat mutation and Save/fresh-reopen observations plus a private output lineage. Exact OplPap/OplQListFormat field deltas, omission/default rules, and raw 0x4A stability require an owner-scoped structural join on the private outputs; COM numeric equality alone does not prove private-wire scalar equality."
    }
    arms = $results
}

$summaryPath = Join-Path $analysisDir "active-list-01.json"
Write-PubJson -Value $summary -Path $summaryPath

$logLines = @(
    "experiment=$ExpectedExperiment",
    "fixture_sha256=$fixtureHash",
    "publisher_exe_sha256=$publisherHash",
    "arm_count=$($results.Count)"
)
foreach ($arm in $results) {
    $logLines += "arm=$($arm.arm) family=$($arm.requested.family) semantic=$($arm.requested.semantic_name) mutation=$($arm.mutation.state) save=$($arm.save.state) reopen=$($arm.reopen.state) roundtrip=$($arm.classification.requested_value_roundtrips)"
}
$logLines | Set-Content -LiteralPath (Join-Path $logDir "active-list-01.txt") -Encoding ASCII
