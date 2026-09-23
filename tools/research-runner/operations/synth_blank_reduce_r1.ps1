param(
    [Parameter(Mandatory = $true)]
    [string]$PacketPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedExperiment = "SYNTH-BLANK-REDUCE-R1"
$ExpectedFixtureSha256 = "5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b"
$ExpectedPublisherExeSha256 = "e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b"

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

Import-Module (Join-Path $PSScriptRoot "CfbStructuredStorage.psm1") -Force
Import-Module (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "windows\pub-runtime\PubRuntime.psm1") -Force

$analysisDir = Join-Path $OutputRoot "analysis"
$logDir = Join-Path $OutputRoot "logs"
$privateDir = Join-Path $OutputRoot "private\synth-blank-reduce-r1"
New-Item -ItemType Directory -Force -Path $analysisDir | Out-Null
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
New-Item -ItemType Directory -Force -Path $privateDir | Out-Null

function Get-DocumentSnapshot {
    param(
        [Parameter(Mandatory = $true)]
        $Document,
        [Parameter(Mandatory = $true)]
        [string]$Phase
    )

    $shapeCount = 0
    for ($pageIndex = 1; $pageIndex -le [int]$Document.Pages.Count; $pageIndex++) {
        $shapeCount += [int]$Document.Pages.Item($pageIndex).Shapes.Count
    }

    return [ordered]@{
        phase = $Phase
        page_count = [int]$Document.Pages.Count
        shape_count = $shapeCount
        name = Get-PubSafeValue { [string]$Document.Name } "Document.Name"
        saved = Get-PubSafeValue { [bool]$Document.Saved } "Document.Saved"
    }
}

function Get-ExceptionRecord {
    param([Parameter(Mandatory = $true)]$Exception)

    $hresult = $null
    if ($Exception.HResult) {
        $hresult = Format-PubHResult ([int]$Exception.HResult)
    }
    return [ordered]@{
        hresult = $hresult
        message = [string]$Exception.Message
    }
}

function Close-ComDocument {
    param($Document)
    if ($null -eq $Document) { return }
    try { $Document.Close() } catch {}
    try { [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Document) } catch {}
}

function Invoke-Arm {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ArmId,
        [Parameter(Mandatory = $true)]
        [array]$Removals
    )

    $armDir = Join-Path $privateDir $ArmId
    New-Item -ItemType Directory -Force -Path $armDir | Out-Null

    $candidatePath = Join-Path $armDir "candidate.pub"
    Copy-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE -Destination $candidatePath
    $beforeHash = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash.ToLowerInvariant()

    $mutationReceipt = @()
    foreach ($removal in $Removals) {
        $storageName = [string]$removal.storage
        $elementName = [string]$removal.element
        $presentBefore = Test-CfbStreamExists -Path $candidatePath -StorageName $storageName -ElementName $elementName
        if (-not $presentBefore) {
            throw "$ArmId target stream absent before mutation: storage='$storageName' element='$elementName'"
        }
        Remove-CfbStream -Path $candidatePath -StorageName $storageName -ElementName $elementName
        $presentAfter = Test-CfbStreamExists -Path $candidatePath -StorageName $storageName -ElementName $elementName
        if ($presentAfter) {
            throw "$ArmId target stream still present after mutation: storage='$storageName' element='$elementName'"
        }
        $mutationReceipt += [ordered]@{
            storage = $storageName
            element = $elementName
            present_before = $presentBefore
            present_after = $presentAfter
        }
    }

    $candidateRecord = Get-PubFileRecord $candidatePath
    if ($candidateRecord.sha256 -eq $beforeHash) {
        throw "$ArmId mutation did not change candidate whole-file SHA-256"
    }

    $record = [ordered]@{
        arm_id = $ArmId
        removals = $mutationReceipt
        candidate = [ordered]@{
            sha256 = $candidateRecord.sha256
            size = [int64]$candidateRecord.size
            private_path = $candidateRecord.path
        }
        pre_save_open = [ordered]@{
            status = "not_run"
            snapshot = $null
            error = $null
        }
        save_as = [ordered]@{
            status = "not_run"
            output = $null
            error = $null
        }
        fresh_reopen = [ordered]@{
            status = "not_run"
            snapshot = $null
            error = $null
        }
        classification = "unresolved"
    }

    $application = $null
    $document = $null
    $savedPath = Join-Path $armDir "native-saveas.pub"
    try {
        $application = New-PubPublisherApplication
        $document = $application.Open($candidatePath, $false, $false)
        $record.pre_save_open.status = "opened"
        $record.pre_save_open.snapshot = Get-DocumentSnapshot -Document $document -Phase "pre_save_open"

        try {
            # pbFilePublication = 1. Keep a new path so the candidate bytes remain immutable evidence.
            $document.SaveAs($savedPath, 1, $false)
            $record.save_as.status = "saved"
            $record.save_as.output = Get-PubFileRecord $savedPath
        }
        catch {
            $record.save_as.status = "error"
            $record.save_as.error = Get-ExceptionRecord $_.Exception
        }
    }
    catch {
        $record.pre_save_open.status = "rejected"
        $record.pre_save_open.error = Get-ExceptionRecord $_.Exception
    }
    finally {
        Close-ComDocument $document
        Close-PubPublisherApplication $application
    }

    if ($record.save_as.status -eq "saved") {
        $reopenApplication = $null
        $reopenDocument = $null
        try {
            $reopenApplication = New-PubPublisherApplication
            $reopenDocument = $reopenApplication.Open($savedPath, $true, $false)
            $record.fresh_reopen.status = "opened"
            $record.fresh_reopen.snapshot = Get-DocumentSnapshot -Document $reopenDocument -Phase "fresh_reopen"
        }
        catch {
            $record.fresh_reopen.status = "rejected"
            $record.fresh_reopen.error = Get-ExceptionRecord $_.Exception
        }
        finally {
            Close-ComDocument $reopenDocument
            Close-PubPublisherApplication $reopenApplication
        }
    }

    if ($record.pre_save_open.status -eq "rejected") {
        $record.classification = "required_or_structurally_coupled_on_open"
    }
    elseif ($record.save_as.status -eq "error") {
        $record.classification = "optional_on_open_but_save_failed"
    }
    elseif ($record.fresh_reopen.status -eq "rejected") {
        $record.classification = "optional_on_open_but_roundtrip_failed"
    }
    elseif ($record.fresh_reopen.status -eq "opened") {
        $allRegenerated = $true
        foreach ($removal in $Removals) {
            if (-not (Test-CfbStreamExists -Path $savedPath -StorageName ([string]$removal.storage) -ElementName ([string]$removal.element))) {
                $allRegenerated = $false
            }
        }
        $record.classification = if ($allRegenerated) {
            "regenerated_on_save"
        } else {
            "optional_through_save_reopen"
        }
    }

    return $record
}

$summary = [ordered]@{
    schema = "pub-synth-blank-reduce-r1/summary/v1"
    experiment_id = $ExpectedExperiment
    fixture = [ordered]@{
        sha256 = $fixtureHash
        size = (Get-Item -LiteralPath $env:PUB_RESEARCH_FIXTURE).Length
    }
    publisher = [ordered]@{
        exe_sha256 = $publisherHash
        expected_version_prefix = "16.0.12527."
    }
    reset = [ordered]@{
        verified_cold_restore_required = $false
        authority_ceiling = "bounded same-runner Publisher 16.0.12527.22145 whole-stream omission discriminator; not cold-reproducibility evidence"
    }
    arms = @()
}

$summary.arms += Invoke-Arm -ArmId "R1-A-omit-envelope" -Removals @(
    [ordered]@{ storage = ""; element = "Envelope" }
)
$summary.arms += Invoke-Arm -ArmId "R1-B-omit-escher-delay" -Removals @(
    [ordered]@{ storage = "Escher"; element = "EscherDelayStm" }
)
$summary.arms += Invoke-Arm -ArmId "R1-C-omit-summary-information" -Removals @(
    [ordered]@{ storage = ""; element = ([string][char]5 + "SummaryInformation") }
)
$summary.arms += Invoke-Arm -ArmId "R1-D-omit-document-summary-information" -Removals @(
    [ordered]@{ storage = ""; element = ([string][char]5 + "DocumentSummaryInformation") }
)
$summary.arms += Invoke-Arm -ArmId "R1-E-omit-both-property-sets" -Removals @(
    [ordered]@{ storage = ""; element = ([string][char]5 + "SummaryInformation") },
    [ordered]@{ storage = ""; element = ([string][char]5 + "DocumentSummaryInformation") }
)

$summaryPath = Join-Path $analysisDir "synth-blank-reduce-r1.json"
$summary | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $summaryPath -Encoding UTF8

$logLines = @(
    "experiment=$ExpectedExperiment",
    "fixture_sha256=$fixtureHash",
    "publisher_exe_sha256=$publisherHash",
    "arms=$($summary.arms.Count)"
)
foreach ($arm in $summary.arms) {
    $logLines += "$($arm.arm_id)=$($arm.classification)"
}
$logLines | Set-Content -LiteralPath (Join-Path $logDir "synth-blank-reduce-r1.txt") -Encoding ASCII
