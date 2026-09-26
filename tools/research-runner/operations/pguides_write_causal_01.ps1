param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedExperiment = "PGUIDES-WRITE-CAUSAL-01"
$ExpectedFixtureName = "REG-TST2-pub97-to-pub2007-with-pub2007.pub"
$ExpectedFixtureSha256 = "28481deeac014133b960e09376bfd31c2ec14bfb2a0679cb3c010855a9eeec29"
$ExpectedPublisherExeSha256 = "e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b"
$PbFilePublication = 1
$GetProperty = [System.Reflection.BindingFlags]::GetProperty
$SetProperty = [System.Reflection.BindingFlags]::SetProperty

$packet = Get-Content -LiteralPath $PacketPath -Raw | ConvertFrom-Json
if ([string]$packet.id -ne $ExpectedExperiment) {
    throw "Unexpected experiment id: $($packet.id)"
}
if ([string]::IsNullOrWhiteSpace([string]$env:PUB_RESEARCH_PUBLISHER_EXE)) {
    throw "PUB_RESEARCH_PUBLISHER_EXE was not resolved by prepare_native_run.ps1."
}
$publisherHash = (Get-FileHash -LiteralPath $env:PUB_RESEARCH_PUBLISHER_EXE -Algorithm SHA256).Hash.ToLowerInvariant()
if ($publisherHash -ne $ExpectedPublisherExeSha256) {
    throw "Publisher executable SHA-256 mismatch: $publisherHash"
}

Import-Module (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "windows\pub-runtime\PubRuntime.psm1") -Force

$analysisDir = Join-Path $OutputRoot "analysis"
$logDir = Join-Path $OutputRoot "logs"
$privateDir = Join-Path $OutputRoot "private\pguides-write-causal-01"
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

function Resolve-ExactFixture {
    $root = [Environment]::ExpandEnvironmentVariables([string]$env:PUB_RESEARCH_FIXTURE_ROOT)
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "PUB_RESEARCH_FIXTURE_ROOT is required for PGUIDES-WRITE-CAUSAL-01."
    }
    if (-not (Test-Path -LiteralPath $root -PathType Container)) {
        throw "PUB_RESEARCH_FIXTURE_ROOT does not exist: $root"
    }

    $nameMatches = @(Get-ChildItem -LiteralPath $root -Recurse -File -Filter $ExpectedFixtureName -ErrorAction Stop)
    $exact = @()
    foreach ($item in $nameMatches) {
        $sha = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($sha -eq $ExpectedFixtureSha256) {
            $exact += $item
        }
    }
    if ($exact.Count -ne 1) {
        throw "Expected exactly one exact REG-TST2 fixture under PUB_RESEARCH_FIXTURE_ROOT; name_matches=$($nameMatches.Count) exact_hash_matches=$($exact.Count)"
    }
    return $exact[0].FullName
}

function Get-AdjustmentCount {
    param([Parameter(Mandatory = $true)]$Shape)
    try {
        return [int]$Shape.Adjustments.Count
    }
    catch {
        return 0
    }
}

function Get-AdjustmentValue {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][int]$Index
    )
    $adjustments = $Shape.Adjustments
    return [double]$adjustments.GetType().InvokeMember(
        "Item",
        $GetProperty,
        $null,
        $adjustments,
        @([int]$Index)
    )
}

function Set-AdjustmentValue {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][int]$Index,
        [Parameter(Mandatory = $true)][double]$Value
    )
    $adjustments = $Shape.Adjustments
    [void]$adjustments.GetType().InvokeMember(
        "Item",
        $SetProperty,
        $null,
        $adjustments,
        @([int]$Index, [single]$Value)
    )
}

function Get-ShapeAtPath {
    param(
        [Parameter(Mandatory = $true)]$Document,
        [Parameter(Mandatory = $true)][int]$PageIndex,
        [Parameter(Mandatory = $true)][int[]]$ShapePath
    )
    if ($ShapePath.Count -lt 1) {
        throw "ShapePath cannot be empty."
    }
    $shape = $Document.Pages.Item($PageIndex).Shapes.Item([int]$ShapePath[0])
    for ($i = 1; $i -lt $ShapePath.Count; $i++) {
        $shape = $shape.GroupItems.Item([int]$ShapePath[$i])
    }
    return $shape
}

function Add-AdjustmentCandidates {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][int]$PageIndex,
        [Parameter(Mandatory = $true)][int[]]$ShapePath,
        [Parameter(Mandatory = $true)][System.Collections.ArrayList]$Output
    )
    $count = Get-AdjustmentCount -Shape $Shape
    if ($count -gt 0) {
        [void]$Output.Add([ordered]@{
            page_index = $PageIndex
            shape_path = @($ShapePath)
            name = Get-PubSafeValue { [string]$Shape.Name } "Shape.Name"
            shape_id = Get-PubSafeValue { [int]$Shape.ID } "Shape.ID"
            shape_type = Get-PubSafeValue { [int]$Shape.Type } "Shape.Type"
            auto_shape_type = Get-PubSafeValue { [int]$Shape.AutoShapeType } "Shape.AutoShapeType"
            adjustment_count = $count
            adjustment_1 = Get-AdjustmentValue -Shape $Shape -Index 1
        })
    }

    $groupCount = 0
    try { $groupCount = [int]$Shape.GroupItems.Count } catch { $groupCount = 0 }
    for ($i = 1; $i -le $groupCount; $i++) {
        $child = $Shape.GroupItems.Item($i)
        Add-AdjustmentCandidates -Shape $child -PageIndex $PageIndex -ShapePath @($ShapePath + $i) -Output $Output
    }
}

function Get-AdjustmentCandidates {
    param([Parameter(Mandatory = $true)]$Document)
    $output = [System.Collections.ArrayList]::new()
    for ($pageIndex = 1; $pageIndex -le [int]$Document.Pages.Count; $pageIndex++) {
        $page = $Document.Pages.Item($pageIndex)
        for ($shapeIndex = 1; $shapeIndex -le [int]$page.Shapes.Count; $shapeIndex++) {
            $shape = $page.Shapes.Item($shapeIndex)
            Add-AdjustmentCandidates -Shape $shape -PageIndex $pageIndex -ShapePath @($shapeIndex) -Output $output
        }
    }
    return @($output)
}

function Get-ShapeSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][string]$Phase
    )
    $count = Get-AdjustmentCount -Shape $Shape
    $values = @()
    for ($i = 1; $i -le $count; $i++) {
        try { $values += Get-AdjustmentValue -Shape $Shape -Index $i } catch { $values += $null }
    }
    return [ordered]@{
        phase = $Phase
        name = Get-PubSafeValue { [string]$Shape.Name } "Shape.Name"
        shape_id = Get-PubSafeValue { [int]$Shape.ID } "Shape.ID"
        shape_type = Get-PubSafeValue { [int]$Shape.Type } "Shape.Type"
        auto_shape_type = Get-PubSafeValue { [int]$Shape.AutoShapeType } "Shape.AutoShapeType"
        left = Get-PubSafeValue { [double]$Shape.Left } "Shape.Left"
        top = Get-PubSafeValue { [double]$Shape.Top } "Shape.Top"
        width = Get-PubSafeValue { [double]$Shape.Width } "Shape.Width"
        height = Get-PubSafeValue { [double]$Shape.Height } "Shape.Height"
        rotation = Get-PubSafeValue { [double]$Shape.Rotation } "Shape.Rotation"
        nodes_count = Get-PubSafeValue { [int]$Shape.Nodes.Count } "Shape.Nodes.Count"
        adjustment_count = $count
        adjustment_values = $values
    }
}

function Get-ProbeValues {
    param([Parameter(Mandatory = $true)][double]$Original)
    $raw = @(
        ($Original * 0.75),
        ($Original * 1.25),
        ($Original - 1.0),
        ($Original + 1.0),
        ($Original * 0.5),
        ($Original * 1.5),
        0.0,
        1.0,
        5400.0,
        10800.0,
        16200.0,
        21600.0
    )
    $unique = @()
    foreach ($value in $raw) {
        if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) { continue }
        if ([Math]::Abs($value - $Original) -lt 0.000001) { continue }
        if (-not ($unique | Where-Object { [Math]::Abs($_ - $value) -lt 0.000001 })) {
            $unique += [double]$value
        }
    }
    return $unique
}

function Find-TwoAdmissibleValues {
    param(
        [Parameter(Mandatory = $true)]$Shape,
        [Parameter(Mandatory = $true)][double]$Original
    )
    $accepted = @()
    foreach ($candidate in @(Get-ProbeValues -Original $Original)) {
        try {
            Set-AdjustmentValue -Shape $Shape -Index 1 -Value $candidate
            $readback = Get-AdjustmentValue -Shape $Shape -Index 1
            Set-AdjustmentValue -Shape $Shape -Index 1 -Value $Original
            $restored = Get-AdjustmentValue -Shape $Shape -Index 1
            if ([Math]::Abs($restored - $Original) -ge 0.0001) {
                throw "Preflight restore did not return to original value: original=$Original restored=$restored"
            }
            if ([Math]::Abs($readback - $Original) -lt 0.0001) { continue }
            if (-not ($accepted | Where-Object { [Math]::Abs($_ - $readback) -lt 0.0001 })) {
                $accepted += [double]$readback
            }
            if ($accepted.Count -ge 2) { break }
        }
        catch {
            try { Set-AdjustmentValue -Shape $Shape -Index 1 -Value $Original } catch {}
        }
    }
    if ($accepted.Count -lt 2) {
        throw "Could not find two distinct admissible Adjustments(1) values from an unsaved preflight."
    }
    return @($accepted[0], $accepted[1])
}

function Invoke-RawProbe {
    param(
        [Parameter(Mandatory = $true)][string]$PubPath,
        [Parameter(Mandatory = $true)][string]$OutputPath
    )
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($null -eq $cargo) {
        return [ordered]@{ state = "deferred_no_cargo"; output = $null }
    }
    & cargo run --quiet --manifest-path tools/research-runner/pguides-probe/Cargo.toml -- $PubPath |
        Set-Content -LiteralPath $OutputPath -Encoding utf8
    if ($LASTEXITCODE -ne 0) {
        throw "pguides-write-probe failed for $PubPath with exit code $LASTEXITCODE"
    }
    return [ordered]@{
        state = "completed"
        output = [IO.Path]::GetFileName($OutputPath)
        sha256 = (Get-FileHash -LiteralPath $OutputPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

$fixturePath = Resolve-ExactFixture
$fixtureReceipt = Get-FileReceipt $fixturePath

$preflightApp = $null
$preflightDoc = $null
try {
    $preflightApp = New-PubPublisherApplication
    $preflightDoc = $preflightApp.Open($fixturePath, $true, $false)
    $candidates = @(Get-AdjustmentCandidates -Document $preflightDoc)
    if ($candidates.Count -lt 1) {
        throw "Exact REG-TST2 opened but no Shape.Adjustments.Count > 0 candidate was found."
    }
    $selected = $candidates[0]
    $preflightShape = Get-ShapeAtPath -Document $preflightDoc -PageIndex ([int]$selected.page_index) -ShapePath ([int[]]$selected.shape_path)
    $original = Get-AdjustmentValue -Shape $preflightShape -Index 1
    $probeValues = @(Find-TwoAdmissibleValues -Shape $preflightShape -Original $original)
    $valueA = [double]$probeValues[0]
    $valueB = [double]$probeValues[1]
}
finally {
    Close-ComDocument $preflightDoc
    Close-PubPublisherApplication $preflightApp
}

function Invoke-Arm {
    param(
        [Parameter(Mandatory = $true)][string]$ArmId,
        [Parameter(Mandatory = $true)][string]$Mode,
        [double]$TargetValue = 0.0
    )

    $armRoot = Join-Path $privateDir $ArmId
    New-Item -ItemType Directory -Force -Path $armRoot | Out-Null
    $inputPath = Join-Path $armRoot "input.pub"
    $outputPath = Join-Path $armRoot "output.pub"
    Copy-Item -LiteralPath $fixturePath -Destination $inputPath

    $arm = [ordered]@{
        arm = $ArmId
        mode = $Mode
        input_pub = Get-FileReceipt $inputPath
        selected = $selected
        original_adjustment_1 = $original
        target_adjustment_1 = if ($Mode -eq "noop") { $null } elseif ($Mode -eq "restore") { $original } else { $TargetValue }
        before_mutation = $null
        after_mutation = $null
        after_save = $null
        reopen = [ordered]@{ state = "not_attempted"; snapshot = $null; error = $null }
        save = [ordered]@{ state = "not_attempted"; output_pub = $null; error = $null }
        mutation = [ordered]@{ state = "not_attempted"; readback = $null; sequence = @(); error = $null }
        raw_probe = $null
    }

    $application = $null
    $document = $null
    try {
        $application = New-PubPublisherApplication
        $document = $application.Open($inputPath, $false, $false)
        $shape = Get-ShapeAtPath -Document $document -PageIndex ([int]$selected.page_index) -ShapePath ([int[]]$selected.shape_path)
        $arm.before_mutation = Get-ShapeSnapshot -Shape $shape -Phase "before_mutation"

        try {
            switch ($Mode) {
                "noop" {
                    $arm.mutation.state = "no_op"
                    $arm.mutation.readback = Get-AdjustmentValue -Shape $shape -Index 1
                }
                "set" {
                    Set-AdjustmentValue -Shape $shape -Index 1 -Value $TargetValue
                    $readback = Get-AdjustmentValue -Shape $shape -Index 1
                    $arm.mutation.state = "set"
                    $arm.mutation.readback = $readback
                    $arm.mutation.sequence = @($readback)
                }
                "restore" {
                    Set-AdjustmentValue -Shape $shape -Index 1 -Value $valueA
                    $afterA = Get-AdjustmentValue -Shape $shape -Index 1
                    Set-AdjustmentValue -Shape $shape -Index 1 -Value $original
                    $afterRestore = Get-AdjustmentValue -Shape $shape -Index 1
                    $arm.mutation.state = "set_then_restore"
                    $arm.mutation.readback = $afterRestore
                    $arm.mutation.sequence = @($afterA, $afterRestore)
                }
                default {
                    throw "Unsupported arm mode: $Mode"
                }
            }
            $arm.after_mutation = Get-ShapeSnapshot -Shape $shape -Phase "after_mutation"
        }
        catch {
            $arm.mutation.state = "error"
            $arm.mutation.error = Get-ExceptionRecord $_.Exception
        }

        if ($arm.mutation.state -ne "error") {
            try {
                $document.SaveAs($outputPath, $PbFilePublication, $false)
                $arm.save.state = "saved"
                $arm.save.output_pub = Get-FileReceipt $outputPath
                $savedShape = Get-ShapeAtPath -Document $document -PageIndex ([int]$selected.page_index) -ShapePath ([int[]]$selected.shape_path)
                $arm.after_save = Get-ShapeSnapshot -Shape $savedShape -Phase "after_save"
            }
            catch {
                $arm.save.state = "error"
                $arm.save.error = Get-ExceptionRecord $_.Exception
            }
        }
    }
    finally {
        Close-ComDocument $document
        Close-PubPublisherApplication $application
    }

    if ($arm.save.state -eq "saved" -and (Test-Path -LiteralPath $outputPath)) {
        $reopenApp = $null
        $reopenDoc = $null
        try {
            $reopenApp = New-PubPublisherApplication
            $reopenDoc = $reopenApp.Open($outputPath, $true, $false)
            $reopenShape = Get-ShapeAtPath -Document $reopenDoc -PageIndex ([int]$selected.page_index) -ShapePath ([int[]]$selected.shape_path)
            $arm.reopen.state = "opened"
            $arm.reopen.snapshot = Get-ShapeSnapshot -Shape $reopenShape -Phase "fresh_reopen"
        }
        catch {
            $arm.reopen.state = "error"
            $arm.reopen.error = Get-ExceptionRecord $_.Exception
        }
        finally {
            Close-ComDocument $reopenDoc
            Close-PubPublisherApplication $reopenApp
        }

        $rawPath = Join-Path $analysisDir ("raw-" + $ArmId.ToLowerInvariant() + ".json")
        $arm.raw_probe = Invoke-RawProbe -PubPath $outputPath -OutputPath $rawPath
    }

    return $arm
}

$sourceRawPath = Join-Path $analysisDir "raw-source.json"
$sourceRaw = Invoke-RawProbe -PubPath $fixturePath -OutputPath $sourceRawPath

$results = @(
    (Invoke-Arm -ArmId "C0" -Mode "noop"),
    (Invoke-Arm -ArmId "A" -Mode "set" -TargetValue $valueA),
    (Invoke-Arm -ArmId "B" -Mode "set" -TargetValue $valueB),
    (Invoke-Arm -ArmId "R" -Mode "restore")
)

$rawStates = @($sourceRaw.state) + @($results | ForEach-Object { if ($null -ne $_.raw_probe) { $_.raw_probe.state } else { "not_run" } })
$rawProbeStatus = [ordered]@{
    schema = "pub-pguides-write-probe-status/v1"
    cargo_available = ($null -ne (Get-Command cargo -ErrorAction SilentlyContinue))
    source = $sourceRaw
    arm_states = @($results | ForEach-Object { [ordered]@{ arm = $_.arm; raw_probe = $_.raw_probe } })
    complete_for_all_outputs = (-not ($rawStates | Where-Object { $_ -ne "completed" }))
    note = "Raw extraction reuses pub-cfb and pub-escher only; no new OfficeArt parser is introduced. If cargo is absent on the Publisher runner, private PUB outputs remain the evidence source and raw join is deferred to a Rust-capable local consumer."
}
Write-PubJson -Value $rawProbeStatus -Path (Join-Path $analysisDir "raw-probe-status.json")

$summary = [ordered]@{
    schema = "pub-pguides-write-causal-01/summary/v1"
    experiment_id = $ExpectedExperiment
    fixture = $fixtureReceipt
    fixture_provenance = [ordered]@{
        canonical_name = $ExpectedFixtureName
        public_source = "fosnola/libmspub-test testset/12.0"
        expected_sha256 = $ExpectedFixtureSha256
    }
    publisher = [ordered]@{
        exe_sha256 = $publisherHash
        expected_version_prefix = "16.0.12527."
    }
    preflight = [ordered]@{
        candidate_count = $candidates.Count
        selected = $selected
        original_adjustment_1 = $original
        admissible_value_a = $valueA
        admissible_value_b = $valueB
        values_chosen_from_unsaved_probe = $true
    }
    evidence_boundary = [ordered]@{
        native_pub_outputs_private = $true
        generic_officeart_parser_added = $false
        raw_probe_reuses_existing_pub_cfb_and_pub_escher = $true
        native_pdf_export_repeated = $false
        prior_native_render_dependency_evidence = "OBS-FOPT-001 / ADJUST-REAL-01 already proves one adjustValue scalar changes native Publisher geometry; this task measures native save/write projection and reopen persistence."
    }
    arms = $results
}

Write-PubJson -Value $summary -Path (Join-Path $analysisDir "pguides-write-causal-01.json")

$logLines = @(
    "experiment=$ExpectedExperiment",
    "fixture_name=$($fixtureReceipt.name)",
    "fixture_sha256=$($fixtureReceipt.sha256)",
    "publisher_exe_sha256=$publisherHash",
    "candidate_count=$($candidates.Count)",
    "selected_page=$($selected.page_index)",
    "selected_path=$([string]::Join('.', [int[]]$selected.shape_path))",
    "original_adjustment_1=$original",
    "value_a=$valueA",
    "value_b=$valueB",
    "raw_probe_complete=$($rawProbeStatus.complete_for_all_outputs)"
)
foreach ($arm in $results) {
    $rawState = if ($null -ne $arm.raw_probe) { [string]$arm.raw_probe.state } else { "not_run" }
    $logLines += "arm=$($arm.arm) mode=$($arm.mode) mutation=$($arm.mutation.state) save=$($arm.save.state) reopen=$($arm.reopen.state) raw=$rawState"
}
$logLines | Set-Content -LiteralPath (Join-Path $logDir "pguides-write-causal-01.txt") -Encoding ASCII
