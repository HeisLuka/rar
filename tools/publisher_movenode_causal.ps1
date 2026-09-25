param(
    [switch]$SelfTest,
    [string]$Source,
    [string]$OutDir,
    [long]$PageId,
    [string]$OracleTag,
    [single]$DeltaPoints = 1.0,
    [string]$TagName = "PUB_ORACLE_ID"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$culture = [System.Globalization.CultureInfo]::InvariantCulture

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Format-Point($Value) {
    return ([single]$Value).ToString("R", $culture)
}

function Release-Com($Value) {
    if ($null -ne $Value -and [System.Runtime.InteropServices.Marshal]::IsComObject($Value)) {
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Value)
    }
}

function Get-TaggedShape($Document, [long]$WantedPageId, [string]$WantedTagName, [string]$WantedTagValue) {
    $page = $null
    try {
        $page = $Document.Pages.FindByPageID($WantedPageId)
        $matches = @()
        for ($shapeIndex = 1; $shapeIndex -le $page.Shapes.Count; $shapeIndex++) {
            $shape = $null
            try {
                $shape = $page.Shapes.Item($shapeIndex)
                $matched = $false
                for ($tagIndex = 1; $tagIndex -le $shape.Tags.Count; $tagIndex++) {
                    $tag = $null
                    try {
                        $tag = $shape.Tags.Item($tagIndex)
                        if ([string]$tag.Name -eq $WantedTagName -and [string]$tag.Value -eq $WantedTagValue) {
                            $matched = $true
                            break
                        }
                    }
                    finally {
                        Release-Com $tag
                    }
                }
                if ($matched) {
                    $matches += [pscustomobject]@{
                        index = $shapeIndex
                        id = [long]$shape.ID
                        name = [string]$shape.Name
                    }
                }
            }
            finally {
                Release-Com $shape
            }
        }

        if ($matches.Count -ne 1) {
            throw "Expected exactly one shape tagged $WantedTagName=$WantedTagValue on PageID=$WantedPageId, found $($matches.Count)"
        }

        $found = $page.Shapes.Item([int]$matches[0].index)
        return [pscustomobject]@{
            page = $page
            shape = $found
            shape_id = [long]$found.ID
            shape_name = [string]$found.Name
        }
    }
    catch {
        Release-Com $page
        throw
    }
}

function Get-Geometry($Shape) {
    return [ordered]@{
        left = Format-Point $Shape.Left
        top = Format-Point $Shape.Top
        width = Format-Point $Shape.Width
        height = Format-Point $Shape.Height
    }
}

function Close-Document($Document) {
    if ($null -ne $Document) {
        try { $Document.Close() } catch {}
        Release-Com $Document
    }
}

function Quit-Publisher($Application) {
    if ($null -ne $Application) {
        try { $Application.Quit() } catch {}
        Release-Com $Application
    }
}

function Open-Document($Application, [string]$Path) {
    return $Application.Open((Resolve-Path -LiteralPath $Path).Path)
}

function Run-Arm(
    [string]$Name,
    [string]$Mode,
    [string]$SourcePath,
    [string]$Root,
    [long]$WantedPageId,
    [string]$WantedTagName,
    [string]$WantedTagValue,
    [single]$Delta
) {
    $armDir = Join-Path $Root $Name
    New-Item -ItemType Directory -Force -Path $armDir | Out-Null
    $working = Join-Path $armDir "working.pub"
    $first = Join-Path $armDir "first-save.pub"
    $second = Join-Path $armDir "second-save.pub"

    Copy-Item -LiteralPath $SourcePath -Destination $working -Force
    $workingBeforeSha = Get-Sha256 $working
    $sourceSha = Get-Sha256 $SourcePath
    if ($workingBeforeSha -ne $sourceSha) {
        throw "$Name working-copy hash differs before Open"
    }

    $app = $null
    $doc = $null
    $page = $null
    $shape = $null
    try {
        $app = New-Object -ComObject Publisher.Application
        $publisherVersion = [string]$app.Version
        $publisherBuild = [string]$app.Build

        $doc = Open-Document $app $working
        $resolved = Get-TaggedShape $doc $WantedPageId $WantedTagName $WantedTagValue
        $page = $resolved.page
        $shape = $resolved.shape
        $before = Get-Geometry $shape
        $shapeIdBefore = [long]$resolved.shape_id
        $shapeNameBefore = [string]$resolved.shape_name

        switch ($Mode) {
            "control" {
            }
            "x" {
                $shape.Left = [single]([single]$shape.Left + $Delta)
            }
            "y" {
                $shape.Top = [single]([single]$shape.Top + $Delta)
            }
            "restore" {
                $originalLeft = [single]$shape.Left
                $originalTop = [single]$shape.Top
                $shape.Left = [single]($originalLeft + $Delta)
                $shape.Top = [single]($originalTop + $Delta)
                $shape.Left = $originalLeft
                $shape.Top = $originalTop
            }
            default {
                throw "Unsupported arm mode: $Mode"
            }
        }

        $afterSet = Get-Geometry $shape
        $doc.Save()
        Close-Document $doc
        $doc = $null
        Release-Com $shape
        $shape = $null
        Release-Com $page
        $page = $null

        Copy-Item -LiteralPath $working -Destination $first -Force
        $firstSha = Get-Sha256 $first

        $doc = Open-Document $app $working
        $resolvedReopen = Get-TaggedShape $doc $WantedPageId $WantedTagName $WantedTagValue
        $page = $resolvedReopen.page
        $shape = $resolvedReopen.shape
        $reopen = Get-Geometry $shape
        $shapeIdReopen = [long]$resolvedReopen.shape_id
        $shapeNameReopen = [string]$resolvedReopen.shape_name

        $doc.Save()
        Close-Document $doc
        $doc = $null
        Release-Com $shape
        $shape = $null
        Release-Com $page
        $page = $null

        Copy-Item -LiteralPath $working -Destination $second -Force
        $secondSha = Get-Sha256 $second

        return [ordered]@{
            name = $Name
            mode = $Mode
            publisher_version = $publisherVersion
            publisher_build = $publisherBuild
            baseline_source_sha256 = $sourceSha
            working_copy_sha256_before_open = $workingBeforeSha
            page_id = $WantedPageId
            oracle_tag = [ordered]@{
                name = $WantedTagName
                value = $WantedTagValue
            }
            shape_identity = [ordered]@{
                shape_id_before = $shapeIdBefore
                shape_id_reopen = $shapeIdReopen
                shape_name_before = $shapeNameBefore
                shape_name_reopen = $shapeNameReopen
            }
            before = $before
            after_set = $afterSet
            reopen = $reopen
            first_save_sha256 = $firstSha
            second_save_sha256 = $secondSha
            first_save_path = (Resolve-Path -LiteralPath $first).Path
            second_save_path = (Resolve-Path -LiteralPath $second).Path
        }
    }
    finally {
        if ($null -ne $shape) { Release-Com $shape }
        if ($null -ne $page) { Release-Com $page }
        if ($null -ne $doc) { Close-Document $doc }
        if ($null -ne $app) { Quit-Publisher $app }
        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
    }
}

if ($SelfTest) {
    $sample = [single]1.25
    if ((Format-Point $sample) -ne "1.25") {
        throw "Invariant point formatter self-test failed"
    }
    $payload = [ordered]@{
        schema = "chaptera.publisher-movenode-causal-local.v1"
        self_test = $true
        emu_per_point = 12700
        tag_name = "PUB_ORACLE_ID"
    }
    $payload | ConvertTo-Json -Depth 5
    exit 0
}

foreach ($required in @("Source", "OutDir", "OracleTag")) {
    if ([string]::IsNullOrWhiteSpace((Get-Variable -Name $required -ValueOnly))) {
        throw "-$required is required"
    }
}
if ($PageId -eq 0) {
    throw "-PageId must be non-zero"
}
if ($DeltaPoints -eq 0) {
    throw "-DeltaPoints must be non-zero"
}

$sourcePath = (Resolve-Path -LiteralPath $Source).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outRoot = (Resolve-Path -LiteralPath $OutDir).Path
$sourceShaBefore = Get-Sha256 $sourcePath
$sourceLength = (Get-Item -LiteralPath $sourcePath).Length

$arms = @(
    (Run-Arm "control" "control" $sourcePath $outRoot $PageId $TagName $OracleTag $DeltaPoints),
    (Run-Arm "move-x" "x" $sourcePath $outRoot $PageId $TagName $OracleTag $DeltaPoints),
    (Run-Arm "move-y" "y" $sourcePath $outRoot $PageId $TagName $OracleTag $DeltaPoints),
    (Run-Arm "restore" "restore" $sourcePath $outRoot $PageId $TagName $OracleTag $DeltaPoints)
)

$versions = @($arms | ForEach-Object { "$($_.publisher_version)/$($_.publisher_build)" } | Sort-Object -Unique)
if ($versions.Count -ne 1) {
    throw "Publisher environment drift across arms: $($versions -join ', ')"
}

$sourceShaAfter = Get-Sha256 $sourcePath
if ($sourceShaAfter -ne $sourceShaBefore) {
    throw "Immutable source changed during experiment"
}

$result = [ordered]@{
    schema = "chaptera.publisher-movenode-causal-local.v1"
    source = [ordered]@{
        sha256 = $sourceShaBefore
        byte_len = $sourceLength
        unchanged = $true
    }
    publisher = [ordered]@{
        version = [string]$arms[0].publisher_version
        build = [string]$arms[0].publisher_build
    }
    target = [ordered]@{
        page_id = $PageId
        tag_name = $TagName
        tag_value = $OracleTag
    }
    delta_points = Format-Point $DeltaPoints
    emu_per_point = 12700
    arms = $arms
    boundaries = [ordered]@{
        disposable_copies_only = $true
        source_pub_immutable = $true
        native_writer_capability_granted = $false
        shape_id_cross_save_stability_assumed = $false
        oracle_tag_is_identity = $true
    }
}

$manifest = Join-Path $outRoot "publisher-movenode-causal.local.json"
$result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $manifest -Encoding utf8
Write-Output $manifest
