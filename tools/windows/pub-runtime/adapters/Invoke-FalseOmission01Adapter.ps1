param(
    [Parameter(Mandatory = $true)]
    [string]$RunContextPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$runtimeRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $runtimeRoot "PubRuntime.psm1") -Force

$MsoFalse = 0
$MsoTrue = -1
$PbFilePublication = 1
$TargetTag = "FALSE_OMISSION_TARGET"

function Get-OracleTagValue {
    param(
        [Parameter(Mandatory = $true)]
        $Shape
    )

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

function Find-FalseOmissionTarget {
    param(
        [Parameter(Mandatory = $true)]
        $Document
    )

    $matches = @()
    for ($pageIndex = 1; $pageIndex -le [int]$Document.Pages.Count; $pageIndex++) {
        $page = $Document.Pages.Item($pageIndex)
        for ($shapeIndex = 1; $shapeIndex -le [int]$page.Shapes.Count; $shapeIndex++) {
            $shape = $page.Shapes.Item($shapeIndex)
            if ((Get-OracleTagValue -Shape $shape) -eq $TargetTag) {
                $matches += [ordered]@{
                    page_index = $pageIndex
                    shape_index = $shapeIndex
                    page = $page
                    shape = $shape
                }
            }
        }
    }

    if ($matches.Count -ne 1) {
        throw "Ожидался ровно один PUB_ORACLE_ID=$TargetTag; найдено: $($matches.Count)"
    }

    return $matches[0]
}

function Assert-FalseOmissionTopology {
    param(
        [Parameter(Mandatory = $true)]
        $Document,
        [Parameter(Mandatory = $true)]
        $Target,
        [Parameter(Mandatory = $true)]
        [string]$Phase
    )

    $pageCount = [int]$Document.Pages.Count
    if ($pageCount -ne 1) {
        throw "$Phase topology: ожидалась ровно одна page, найдено: $pageCount"
    }

    if ([int]$Target.page_index -ne 1 -or [int]$Target.shape_index -ne 1) {
        throw "$Phase topology: target должен быть page[1]/shape[1], фактически page[$($Target.page_index)]/shape[$($Target.shape_index)]"
    }

    $shapeCount = [int]$Document.Pages.Item(1).Shapes.Count
    if ($shapeCount -ne 1) {
        throw "$Phase topology: ожидалась ровно одна top-level shape, найдено: $shapeCount"
    }

    return [ordered]@{
        phase = $Phase
        pages_count = $pageCount
        target_page_index = [int]$Target.page_index
        target_shape_index = [int]$Target.shape_index
        target_page_shapes_count = $shapeCount
        unique_top_level_target = $true
    }
}

function Get-FalseOmissionSnapshot {
    param(
        [Parameter(Mandatory = $true)]
        $Target,
        [Parameter(Mandatory = $true)]
        [string]$Phase
    )

    $shape = $Target.shape
    $page = $Target.page

    $textState = [ordered]@{
        state = "not_applicable"
    }
    try {
        if ([int]$shape.HasTextFrame -ne 0) {
            $textState = Get-PubSafeValue { [string]$shape.TextFrame.TextRange.Text } "TextFrame.TextRange.Text"
        }
    }
    catch {
        $textState = [ordered]@{
            state = "error"
            member = "TextFrame.TextRange.Text"
            hresult = if ($_.Exception.HResult) { Format-PubHResult ([int]$_.Exception.HResult) } else { $null }
            message = $_.Exception.Message
        }
    }

    return [ordered]@{
        phase = $Phase
        page_shapes_count = Get-PubSafeValue { [int]$page.Shapes.Count } "Page.Shapes.Count"
        page_index = [int]$Target.page_index
        page_id = Get-PubSafeValue { [int]$page.PageID } "Page.PageID"
        shape_index = [int]$Target.shape_index
        shape_id = Get-PubSafeValue { [int]$shape.ID } "Shape.ID"
        shape_name = Get-PubSafeValue { [string]$shape.Name } "Shape.Name"
        oracle_tag = Get-OracleTagValue -Shape $shape
        fill_visible = Get-PubSafeValue { [int]$shape.Fill.Visible } "Shape.Fill.Visible"
        line_visible = Get-PubSafeValue { [int]$shape.Line.Visible } "Shape.Line.Visible"
        fill_rgb = Get-PubSafeValue { [int]$shape.Fill.ForeColor.RGB } "Shape.Fill.ForeColor.RGB"
        line_rgb = Get-PubSafeValue { [int]$shape.Line.ForeColor.RGB } "Shape.Line.ForeColor.RGB"
        left = Get-PubSafeValue { [double]$shape.Left } "Shape.Left"
        top = Get-PubSafeValue { [double]$shape.Top } "Shape.Top"
        width = Get-PubSafeValue { [double]$shape.Width } "Shape.Width"
        height = Get-PubSafeValue { [double]$shape.Height } "Shape.Height"
        rotation = Get-PubSafeValue { [double]$shape.Rotation } "Shape.Rotation"
        text = $textState
    }
}

function Close-FalseOmissionDocument {
    param(
        $Document,
        $Application
    )

    if ($null -ne $Document) {
        try {
            $Document.Close()
        }
        catch {
            # Cleanup не заменяет зафиксированный outcome.
        }

        try {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Document)
        }
        catch {
            # COM cleanup выполняется по возможности.
        }
    }

    Close-PubPublisherApplication $Application
}

$context = Get-Content -LiteralPath $RunContextPath -Raw | ConvertFrom-Json
$caseId = [string]$context.case_id
$sourceRecord = Get-PubFileRecord ([string]$context.source_pub)

$operation = $null
$candidate = $null
switch ($caseId) {
    "resave-control--current" {
        $operation = "resave-control"
    }
    "fill-visible-true--current" {
        $operation = "fill-visible-true"
        $candidate = [ordered]@{
            owner_class = "OplOdpoFillStyle"
            field_id = "0x03C"
            publisher_name = "FFilled"
            status = "hypothesis_only_until_raw_diff"
        }
    }
    "line-visible-true--current" {
        $operation = "line-visible-true"
        $candidate = [ordered]@{
            owner_class = "OplOdpoLineStyle"
            field_id = "0x03D"
            publisher_name = "FLine"
            status = "hypothesis_only_until_raw_diff"
        }
    }
    default {
        throw "Неизвестный FALSE-OMISSION-01 case_id: $caseId"
    }
}

$result = [ordered]@{
    schema = "pub-false-omission-01/adapter/v1"
    experiment_id = [string]$context.experiment_id
    case_id = $caseId
    source_sha256 = $sourceRecord.sha256
    operation = $operation
    candidate_internal_coordinate = $candidate
    publisher = $null
    identity_contract = [ordered]@{
        source = $null
        after = $null
        reopen = $null
        requirement = "Pages.Count=1; page[1].Shapes.Count=1; target=page[1]/shape[1]"
    }
    before = $null
    mutation = [ordered]@{
        state = "not_attempted"
        hresult = $null
        message = $null
    }
    after = $null
    save = [ordered]@{
        state = "not_attempted"
        path = $null
        sha256 = $null
        size = $null
        hresult = $null
        message = $null
    }
    reopen = $null
    reopen_validation = [ordered]@{
        state = "not_attempted"
        message = $null
    }
    interpretation_guardrails = @(
        "Shape.Fill.Visible не считается OplOdpoFillStyle.FFilled без offline raw attribution.",
        "Shape.Line.Visible не считается OplOdpoLineStyle.FLine без offline raw attribution.",
        "Каждый mutation arm стартует из одних exact source bytes.",
        "resave-control обязателен для отделения SaveAs normalization от semantic mutation.",
        "Current Publisher output не является Publisher 11 writer evidence.",
        "Reopen semantic equality не является byte equality.",
        "Одна page + одна top-level shape с уникальным tag сильно сужает identity bridge, но сама по себе не доказывает raw owner identity.",
        "Adapter не интерпретирует nested OPL bytes."
    )
}

$application = $null
$document = $null

try {
    $application = New-PubPublisherApplication
    $result.publisher = [ordered]@{
        version = Get-PubSafeValue { [string]$application.Version } "Application.Version"
        build = Get-PubSafeValue { [string]$application.Build } "Application.Build"
        path = Get-PubSafeValue { [string]$application.Path } "Application.Path"
    }

    $document = $application.Open([string]$context.source_pub, $false, $false)
    $target = Find-FalseOmissionTarget -Document $document
    $result.identity_contract.source = Assert-FalseOmissionTopology -Document $document -Target $target -Phase "source"
    $shape = $target.shape

    $baselineFillVisible = [int]$shape.Fill.Visible
    $baselineLineVisible = [int]$shape.Line.Visible
    if ($baselineFillVisible -ne $MsoFalse -or $baselineLineVisible -ne $MsoFalse) {
        throw "Fixture contract нарушен: baseline Fill.Visible=$baselineFillVisible Line.Visible=$baselineLineVisible; ожидалось 0/0"
    }

    $result.before = Get-FalseOmissionSnapshot -Target $target -Phase "before"

    try {
        switch ($operation) {
            "resave-control" {
                $result.mutation.state = "control_no_mutation"
            }
            "fill-visible-true" {
                $shape.Fill.Visible = $MsoTrue
                if ([int]$shape.Fill.Visible -ne $MsoTrue) {
                    throw "Fill.Visible setter не дал msoTrue"
                }
                if ([int]$shape.Line.Visible -ne $MsoFalse) {
                    throw "Fill arm неожиданно изменил Line.Visible"
                }
                $result.mutation.state = "ok"
            }
            "line-visible-true" {
                $shape.Line.Visible = $MsoTrue
                if ([int]$shape.Line.Visible -ne $MsoTrue) {
                    throw "Line.Visible setter не дал msoTrue"
                }
                if ([int]$shape.Fill.Visible -ne $MsoFalse) {
                    throw "Line arm неожиданно изменил Fill.Visible"
                }
                $result.mutation.state = "ok"
            }
        }
    }
    catch {
        $result.mutation.state = "error"
        if ($_.Exception.HResult) {
            $result.mutation.hresult = Format-PubHResult ([int]$_.Exception.HResult)
        }
        $result.mutation.message = $_.Exception.Message
    }

    $result.identity_contract.after = Assert-FalseOmissionTopology -Document $document -Target $target -Phase "after"
    $result.after = Get-FalseOmissionSnapshot -Target $target -Phase "after"

    if ($result.mutation.state -eq "ok" -or $result.mutation.state -eq "control_no_mutation") {
        $outputPath = Join-Path ([string]$context.output_dir) ("false-omission-{0}.pub" -f $caseId)
        try {
            $document.SaveAs($outputPath, $PbFilePublication, $false)
            $record = Get-PubFileRecord $outputPath
            $result.save.state = "ok"
            $result.save.path = $record.path
            $result.save.sha256 = $record.sha256
            $result.save.size = $record.size
        }
        catch {
            $result.save.state = "error"
            if ($_.Exception.HResult) {
                $result.save.hresult = Format-PubHResult ([int]$_.Exception.HResult)
            }
            $result.save.message = $_.Exception.Message
        }
    }
}
finally {
    Close-FalseOmissionDocument -Document $document -Application $application
}

if ($result.save.state -eq "ok") {
    $reopenApplication = $null
    $reopenDocument = $null

    try {
        $reopenApplication = New-PubPublisherApplication
        $reopenDocument = $reopenApplication.Open([string]$result.save.path, $true, $false)
        $reopenTarget = Find-FalseOmissionTarget -Document $reopenDocument
        $result.identity_contract.reopen = Assert-FalseOmissionTopology -Document $reopenDocument -Target $reopenTarget -Phase "reopen"
        $result.reopen = Get-FalseOmissionSnapshot -Target $reopenTarget -Phase "reopen"

        $reopenFill = [int]$reopenTarget.shape.Fill.Visible
        $reopenLine = [int]$reopenTarget.shape.Line.Visible

        switch ($operation) {
            "resave-control" {
                if ($reopenFill -ne $MsoFalse -or $reopenLine -ne $MsoFalse) {
                    throw "resave-control reopen нарушил 0/0 visibility contract"
                }
            }
            "fill-visible-true" {
                if ($reopenFill -ne $MsoTrue -or $reopenLine -ne $MsoFalse) {
                    throw "fill arm reopen expected Fill=True/Line=False, got $reopenFill/$reopenLine"
                }
            }
            "line-visible-true" {
                if ($reopenFill -ne $MsoFalse -or $reopenLine -ne $MsoTrue) {
                    throw "line arm reopen expected Fill=False/Line=True, got $reopenFill/$reopenLine"
                }
            }
        }

        $result.reopen_validation.state = "ok"
    }
    catch {
        $result.reopen_validation.state = "error"
        $result.reopen_validation.message = $_.Exception.Message
        if ($null -eq $result.reopen) {
            $result.reopen = [ordered]@{
                phase = "reopen"
                state = "error"
                hresult = if ($_.Exception.HResult) { Format-PubHResult ([int]$_.Exception.HResult) } else { $null }
                message = $_.Exception.Message
            }
        }
    }
    finally {
        Close-FalseOmissionDocument -Document $reopenDocument -Application $reopenApplication
    }
}

Write-PubJson -Value $result -Path (Join-Path ([string]$context.oracle_dir) "false-omission.json")
