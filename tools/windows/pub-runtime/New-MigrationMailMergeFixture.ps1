param(
    [string]$OutputRoot = ".\out\migration-mailmerge"
)

$ErrorActionPreference = "Stop"

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-ArtifactRecord([string]$Path) {
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        file_name = $item.Name
        byte_len = [int64]$item.Length
        sha256 = Get-Sha256 $item.FullName
    }
}

function Release-ComObject($Value) {
    if ($null -ne $Value -and [Runtime.InteropServices.Marshal]::IsComObject($Value)) {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Value)
    }
}

function Get-VisibleTexts($Document) {
    $texts = New-Object System.Collections.Generic.List[string]
    foreach ($page in @($Document.Pages)) {
        foreach ($shape in @($page.Shapes)) {
            try {
                if ($shape.HasTextFrame -eq -1) {
                    $text = [string]$shape.TextFrame.TextRange.Text
                    if (-not [string]::IsNullOrWhiteSpace($text)) {
                        $texts.Add($text.Trim())
                    }
                }
            } catch {
                # Non-text shapes are expected; ignore only text-frame probing failures.
            }
        }
    }
    return @($texts)
}

$root = [IO.Path]::GetFullPath($OutputRoot)
Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $root | Out-Null

$dataPath = Join-Path $root "merge-source.xlsx"
$templatePath = Join-Path $root "mail-merge-template.pub"
$receiptPath = Join-Path $root "mail-merge-producer-receipt.json"

$excel = $null
$workbook = $null
$sheet = $null
$publisher = $null
$document = $null
$catalogArea = $null
$textbox = $null

try {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $workbook = $excel.Workbooks.Add()
    $sheet = $workbook.Worksheets.Item(1)
    $sheet.Name = "Sheet1"
    $sheet.Cells.Item(1, 1).Value2 = "TextValue"
    $sheet.Cells.Item(2, 1).Value2 = "TEXT_A"
    $sheet.Cells.Item(3, 1).Value2 = "TEXT_B"
    $sheet.Cells.Item(4, 1).Value2 = "TEXT_C"
    $workbook.SaveAs($dataPath, 51)
    $workbook.Close($false)
    Release-ComObject $sheet
    $sheet = $null
    Release-ComObject $workbook
    $workbook = $null
    $excel.Quit()
    Release-ComObject $excel
    $excel = $null

    $publisher = New-Object -ComObject Publisher.Application
    $publisherVersion = [string]$publisher.Version
    $publisherBuild = [string]$publisher.Build

    if (-not $publisherVersion.StartsWith("16.0")) {
        throw "Publisher version mismatch: expected 16.0*, got $publisherVersion"
    }
    if (-not $publisherBuild.StartsWith("12527")) {
        throw "Publisher build mismatch: expected 12527*, got $publisherBuild"
    }

    $document = $publisher.NewDocument(161, 1)
    $page = $document.Pages.Item(1)
    $catalogArea = $page.Shapes.AddCatalogMergeArea()
    $textbox = $page.Shapes.AddTextbox(1, 72, 72, 300, 54)
    [void]$textbox.AddToCatalogMergeArea()

    $document.MailMerge.OpenDataSource($dataPath, "", "Sheet1$", 0, -1)

    $range = $textbox.TextFrame.TextRange
    $range.Text = "RESCUE_MAIL_MERGE: "
    [void]$range.InsertAfter("")
    $range = $textbox.TextFrame.TextRange
    [void]$range.InsertMailMergeField("TextValue")

    $dataFields = @()
    foreach ($field in @($document.MailMerge.DataSource.DataFields)) {
        $dataFields += [string]$field.Name
    }
    if ($dataFields -notcontains "TextValue") {
        throw "MailMerge datasource is missing required field TextValue"
    }
    if (-not [bool]$document.IsDataSourceConnected) {
        throw "Publisher does not report an attached datasource"
    }

    $document.SaveAs($templatePath)

    $outputs = @()
    foreach ($count in @(1, 2, 3)) {
        $document.MailMerge.DataSource.FirstRecord = 1
        $document.MailMerge.DataSource.LastRecord = $count
        $merged = $document.MailMerge.Execute($false, 2, "")
        if ($null -eq $merged) {
            throw "MailMerge.Execute returned no publication for record_count=$count"
        }

        try {
            $outputPath = Join-Path $root ("mail-merge-{0}.pub" -f $count)
            $merged.SaveAs($outputPath)
            $observed = @(Get-VisibleTexts $merged)
            $expected = @("TEXT_A", "TEXT_B", "TEXT_C")[0..($count - 1)]
            $joined = $observed -join "`n"
            foreach ($value in $expected) {
                if (-not $joined.Contains($value)) {
                    throw "Merged output $count is missing expected visible text $value"
                }
            }

            $outputs += [ordered]@{
                record_count = $count
                expected_values = @($expected)
                observed_text = @($observed)
                artifact = Get-ArtifactRecord $outputPath
            }
        } finally {
            $merged.Close()
            Release-ComObject $merged
        }
    }

    $receipt = [ordered]@{
        receipt_version = "chaptera.migration-mailmerge-producer.v1"
        producer = "New-MigrationMailMergeFixture.ps1"
        publisher = [ordered]@{
            version = $publisherVersion
            build = $publisherBuild
        }
        fixture = [ordered]@{
            wizard_id = 161
            design = 1
            merge_destination = 2
            merge_field = "TextValue"
            merge_values = @("TEXT_A", "TEXT_B", "TEXT_C")
            data_source_table = 'Sheet1$'
        }
        data_source = Get-ArtifactRecord $dataPath
        template = Get-ArtifactRecord $templatePath
        template_data_source_connected = $true
        data_fields = @($dataFields)
        outputs = @($outputs)
        lineage = "new_controlled_fixture_2026_09_26"
        byte_identity_with_lost_merge_clone_01 = $false
    }

    $receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $receiptPath -Encoding UTF8
    Write-Host "PASS: controlled Publisher mail-merge fixture created."
    Write-Host "Receipt: $receiptPath"
}
finally {
    if ($null -ne $document) {
        try { $document.Close() } catch {}
    }
    if ($null -ne $publisher) {
        try { $publisher.Quit() } catch {}
    }
    Release-ComObject $textbox
    Release-ComObject $catalogArea
    Release-ComObject $document
    Release-ComObject $publisher
    if ($null -ne $workbook) { try { $workbook.Close($false) } catch {} }
    if ($null -ne $excel) { try { $excel.Quit() } catch {} }
    Release-ComObject $sheet
    Release-ComObject $workbook
    Release-ComObject $excel
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}
