param(
    [switch]$PrepareFixturesOnly,
    [switch]$InteractiveResolver
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$AndroidApp = Join-Path $Root "apps\chaptera-mobile-android"
$JniOut = Join-Path $AndroidApp "app\src\main\jniLibs"
$AssetDir = Join-Path $AndroidApp "app\src\androidTest\assets"
$ReceiptDir = Join-Path $Root "artifacts\mobile-reader-v0-device"

New-Item -ItemType Directory -Force -Path $AssetDir, $ReceiptDir | Out-Null

function File-Sha256([string]$Path) {
    return (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToLowerInvariant()
}

$Fixtures = @(
    @{
        Name = "Simple.pub"
        Sha256 = "2606f530052a818c7949b88c9bf245dbc0a74ddc6b5a60004f37b387fe4b955d"
        Url = "https://raw.githubusercontent.com/apache/poi/942d95d85b15d0dfdb3bc9ba1b4f273f277757c8/test-data/publisher/Simple.pub"
    },
    @{
        Name = "SampleBrochure.pub"
        Sha256 = "ffed034ac87e679f0bd08ff9cf74ad11c0e0e510a42b1bc1a7502415f6c29c87"
        Url = "https://raw.githubusercontent.com/apache/poi/942d95d85b15d0dfdb3bc9ba1b4f273f277757c8/test-data/publisher/SampleBrochure.pub"
    },
    @{
        Name = "SampleNewsletter.pub"
        Sha256 = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
        Url = "https://raw.githubusercontent.com/apache/poi/942d95d85b15d0dfdb3bc9ba1b4f273f277757c8/test-data/publisher/SampleNewsletter.pub"
    }
)

function Assert-Fixtures([bool]$AllowDownload) {
    foreach ($fixture in $Fixtures) {
        $path = Join-Path $AssetDir $fixture.Name
        if (-not (Test-Path $path)) {
            if (-not $AllowDownload) {
                throw "MISSING_PINNED_FIXTURE:$($fixture.Name). Run once with -PrepareFixturesOnly before enabling airplane mode."
            }
            Invoke-WebRequest -UseBasicParsing -Uri $fixture.Url -OutFile $path
        }
        $actual = File-Sha256 $path
        if ($actual -ne $fixture.Sha256) {
            throw "FIXTURE_HASH_MISMATCH:$($fixture.Name):$actual"
        }
        Write-Host "Fixture OK: $($fixture.Name) $actual"
    }
}

if ($PrepareFixturesOnly) {
    Assert-Fixtures $true
    Write-Host "Pinned fixtures prepared. Enable airplane mode on the physical device, connect exactly one authorized device, then rerun without -PrepareFixturesOnly."
    exit 0
}

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "MISSING_TOOL:$Name"
    }
}

foreach ($tool in @("adb", "cargo", "cargo-ndk", "gradle", "java")) {
    Require-Command $tool
}

$SdkRoot = @($env:ANDROID_SDK_ROOT, $env:ANDROID_HOME) |
    Where-Object { $_ -and (Test-Path $_) } |
    Select-Object -First 1
if (-not $SdkRoot) {
    throw "ANDROID_SDK_ROOT_MISSING"
}

$NdkRoot = $null
if ($env:ANDROID_NDK_HOME -and (Test-Path $env:ANDROID_NDK_HOME)) {
    $NdkRoot = $env:ANDROID_NDK_HOME
} else {
    $NdkParent = Join-Path $SdkRoot "ndk"
    if (Test-Path $NdkParent) {
        $NdkRoot = Get-ChildItem -Directory $NdkParent |
            Sort-Object Name -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
}
if (-not $NdkRoot) {
    throw "ANDROID_NDK_MISSING"
}

Assert-Fixtures $false

function Invoke-AdbRaw {
    param([string[]]$Args)
    $output = & adb @Args 2>&1
    $exitCode = $LASTEXITCODE
    $text = (($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine).Trim()
    return [pscustomobject]@{
        ExitCode = $exitCode
        Text = $text
    }
}

function Invoke-AdbText {
    param([string[]]$Args)
    $result = Invoke-AdbRaw $Args
    if ($result.ExitCode -ne 0) {
        throw "ADB_FAILED:$($Args -join ' '):$($result.ExitCode)"
    }
    return $result.Text
}

function Test-ChapteraUsefulPage {
    param(
        [string]$DeviceSerial,
        [string]$ExpectedName
    )

    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        $dump = Invoke-AdbRaw @("-s", $DeviceSerial, "shell", "uiautomator", "dump", "/sdcard/chaptera-window.xml")
        if ($dump.ExitCode -eq 0) {
            $xml = Invoke-AdbRaw @("-s", $DeviceSerial, "shell", "cat", "/sdcard/chaptera-window.xml")
            Invoke-AdbRaw @("-s", $DeviceSerial, "shell", "rm", "-f", "/sdcard/chaptera-window.xml") | Out-Null
            if ($xml.ExitCode -eq 0) {
                $hasDocument = $xml.Text -match [regex]::Escape($ExpectedName)
                $hasPage = $xml.Text -match "page 1/"
                $hasLocalOpen = $xml.Text -match "offline local open"
                if ($hasDocument -and $hasPage -and $hasLocalOpen) {
                    return $true
                }
            }
        }
        Start-Sleep -Seconds 1
    }

    return $false
}

$deviceRows = @(& adb devices 2>$null) |
    Select-Object -Skip 1 |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_ -match "\sdevice$" }

if ($deviceRows.Count -ne 1) {
    throw "ANDROID_DEVICE_COUNT:$($deviceRows.Count). Expected exactly one authorized physical Android device."
}

$Serial = ($deviceRows[0] -split "\s+")[0]
$Model = Invoke-AdbText @("-s", $Serial, "shell", "getprop", "ro.product.model")
$ApiRaw = Invoke-AdbText @("-s", $Serial, "shell", "getprop", "ro.build.version.sdk")
$Abi = Invoke-AdbText @("-s", $Serial, "shell", "getprop", "ro.product.cpu.abi")
$Airplane = Invoke-AdbText @("-s", $Serial, "shell", "settings", "get", "global", "airplane_mode_on")

[int]$Api = 0
if (-not [int]::TryParse($ApiRaw, [ref]$Api)) {
    throw "ANDROID_API_UNREADABLE:$ApiRaw"
}
if ($Api -lt 26) {
    throw "UNSUPPORTED_ANDROID_API:$Api"
}
if ($Airplane -ne "1") {
    throw "AIRPLANE_MODE_REQUIRED:observed=$Airplane"
}

$NdkTarget = switch ($Abi) {
    "arm64-v8a" { "arm64-v8a" }
    "x86_64" { "x86_64" }
    default { throw "UNSUPPORTED_ANDROID_ABI:$Abi" }
}

Write-Host "Device eligibility: model=$Model api=$Api abi=$Abi"
Write-Host "Android SDK: $SdkRoot"
Write-Host "Android NDK: $NdkRoot"
Write-Host "Building JNI for $NdkTarget"

if (Test-Path $JniOut) {
    Remove-Item -Recurse -Force $JniOut
}

& cargo +1.94.1 ndk -t $NdkTarget -o $JniOut build --manifest-path (Join-Path $Root "crates\chaptera-mobile-reader-jni\Cargo.toml") --release
if ($LASTEXITCODE -ne 0) {
    throw "JNI_BUILD_FAILED:$LASTEXITCODE"
}

Write-Host "Building application and instrumentation APKs"
& gradle -p $AndroidApp --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest
if ($LASTEXITCODE -ne 0) {
    throw "ANDROID_BUILD_FAILED:$LASTEXITCODE"
}

& adb -s $Serial shell svc wifi disable | Out-Null
& adb -s $Serial shell svc data disable | Out-Null

$StartUtc = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")

Write-Host "Running end-to-end V0 user cycle"
& gradle -p $AndroidApp --no-daemon :app:connectedDebugAndroidTest "-Pandroid.testInstrumentationRunnerArguments.class=com.chaptera.reader.V0UserCycleInstrumentedTest"
$V0Exit = $LASTEXITCODE

$ResolverExit = 99
$ResolverState = "not_run_v0_failed"
$ResolverMime = ""
$ResolverChapteraOffered = $false
$ResolverUnqualifiedIntent = $false
$ResolverUsefulPageVisible = $false

if ($V0Exit -eq 0) {
    $ResolverName = "ChapteraResolverProbe.pub"
    $DeviceResolverPath = "/sdcard/Download/$ResolverName"
    $ResolverSource = Join-Path $AssetDir "SampleNewsletter.pub"

    Write-Host "Staging resolver probe in Android Downloads"
    $push = Invoke-AdbRaw @("-s", $Serial, "push", $ResolverSource, $DeviceResolverPath)
    if ($push.ExitCode -ne 0) {
        $ResolverExit = 1
        $ResolverState = "resolver_stage_failed"
    } else {
        $DocumentId = [System.Uri]::EscapeDataString("primary:Download/$ResolverName")
        $ResolverUri = "content://com.android.externalstorage.documents/document/$DocumentId"
        $query = Invoke-AdbRaw @(
            "-s", $Serial, "shell", "content", "query",
            "--uri", $ResolverUri,
            "--projection", "_display_name:mime_type"
        )

        if ($query.ExitCode -ne 0) {
            $ResolverExit = 2
            $ResolverState = "resolver_provider_query_failed"
        } else {
            $mimeMatch = [regex]::Match($query.Text, "mime_type=([^,\r\n]+)")
            if (-not $mimeMatch.Success) {
                $ResolverExit = 3
                $ResolverState = "resolver_mime_missing"
            } else {
                $ResolverMime = $mimeMatch.Groups[1].Value.Trim()
                Write-Host "Stock document provider MIME for .pub: $ResolverMime"

                $handlers = Invoke-AdbRaw @(
                    "-s", $Serial, "shell", "cmd", "package", "query-activities", "--brief",
                    "-a", "android.intent.action.VIEW",
                    "-c", "android.intent.category.DEFAULT",
                    "-d", $ResolverUri,
                    "-t", $ResolverMime
                )

                if ($handlers.ExitCode -ne 0 -or $handlers.Text -notmatch "com\.chaptera\.reader") {
                    $ResolverExit = 4
                    $ResolverState = "resolver_not_offered"
                } else {
                    $ResolverChapteraOffered = $true
                    $ResolverUnqualifiedIntent = $true
                    Write-Host "Chaptera is offered by the Android resolver for MIME $ResolverMime"
                    $launch = Invoke-AdbRaw @(
                        "-s", $Serial, "shell", "am", "start", "-W",
                        "--grant-read-uri-permission",
                        "-a", "android.intent.action.VIEW",
                        "-c", "android.intent.category.DEFAULT",
                        "-d", $ResolverUri,
                        "-t", $ResolverMime
                    )

                    if ($launch.ExitCode -ne 0) {
                        $ResolverExit = 5
                        $ResolverState = "resolver_launch_failed"
                    } else {
                        Start-Sleep -Seconds 2
                        $activities = Invoke-AdbRaw @("-s", $Serial, "shell", "dumpsys", "activity", "activities")
                        $resumed = (($activities.Text -split "\r?\n") |
                            Where-Object { $_ -match "mResumedActivity|topResumedActivity" }) -join " | "

                        if ($resumed -match "com\.chaptera\.reader") {
                            $ResolverUsefulPageVisible = Test-ChapteraUsefulPage -DeviceSerial $Serial -ExpectedName $ResolverName
                            if ($ResolverUsefulPageVisible) {
                                $ResolverExit = 0
                                $ResolverState = "chaptera_useful_page_visible"
                            } else {
                                $ResolverExit = 8
                                $ResolverState = "resolver_chaptera_no_useful_page"
                            }
                        } elseif ($InteractiveResolver) {
                            Write-Host "Android chooser is active or Chaptera is not yet resumed."
                            Write-Host "On the device, choose Chaptera Reader from the system Open with UI and wait until the PUB page is visible."
                            [void](Read-Host "Press Enter after Chaptera Reader is visible")
                            Start-Sleep -Seconds 1
                            $activities = Invoke-AdbRaw @("-s", $Serial, "shell", "dumpsys", "activity", "activities")
                            $resumed = (($activities.Text -split "\r?\n") |
                                Where-Object { $_ -match "mResumedActivity|topResumedActivity" }) -join " | "
                            if ($resumed -match "com\.chaptera\.reader") {
                                $ResolverUsefulPageVisible = Test-ChapteraUsefulPage -DeviceSerial $Serial -ExpectedName $ResolverName
                                if ($ResolverUsefulPageVisible) {
                                    $ResolverExit = 0
                                    $ResolverState = "chaptera_useful_page_visible_after_human_choice"
                                } else {
                                    $ResolverExit = 8
                                    $ResolverState = "resolver_chaptera_no_useful_page"
                                }
                            } else {
                                $ResolverExit = 6
                                $ResolverState = "resolver_human_choice_not_confirmed"
                            }
                        } else {
                            $ResolverExit = 7
                            $ResolverState = "resolver_requires_human_choice"
                        }
                    }
                }
            }
        }
    }

    Invoke-AdbRaw @("-s", $Serial, "shell", "rm", "-f", $DeviceResolverPath) | Out-Null
}

$PerfExit = 99
if ($V0Exit -eq 0) {
    Write-Host "Running physical performance receipt"
    & gradle -p $AndroidApp --no-daemon :app:connectedDebugAndroidTest "-Pandroid.testInstrumentationRunnerArguments.class=com.chaptera.reader.PhysicalPerfInstrumentedTest"
    $PerfExit = $LASTEXITCODE
}

$EndUtc = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ")
$wifiMatch = & adb -s $Serial shell dumpsys wifi 2>$null | Select-String -Pattern "Wi-Fi is|wifi state" | Select-Object -First 1
$dataMatch = & adb -s $Serial shell dumpsys telephony.registry 2>$null | Select-String -Pattern "mDataConnectionState|mDataConnectionNetworkType" | Select-Object -First 1
$WifiObservation = if ($null -ne $wifiMatch) { $wifiMatch.Line } else { "" }
$DataObservation = if ($null -ne $dataMatch) { $dataMatch.Line } else { "" }

$packageDump = Invoke-AdbRaw @("-s", $Serial, "shell", "dumpsys", "package", "com.chaptera.reader")
if ($packageDump.ExitCode -eq 0 -and $packageDump.Text -match "android\.permission\.INTERNET") {
    throw "INTERNET_PERMISSION_PRESENT"
}

$Stamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssZ")
$PerfReceipt = Join-Path $ReceiptDir "perf-$Stamp.json"
$Receipt = Join-Path $ReceiptDir "receipt-$Stamp.json"

if ($V0Exit -eq 0 -and $PerfExit -eq 0) {
    $perfText = & adb -s $Serial exec-out run-as com.chaptera.reader cat files/chaptera-mobile-perf.json 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $perfText) {
        $PerfExit = 98
    } else {
        ($perfText -join [Environment]::NewLine) | Set-Content -Encoding UTF8 $PerfReceipt
        Write-Host "Performance receipt: $PerfReceipt"
    }
}

$receiptObject = [ordered]@{
    schema = "chaptera.mobile-reader-v0.device-receipt.v1"
    device = [ordered]@{
        model = $Model
        api = $Api
        abi = $Abi
        airplane_mode_enabled = ($Airplane -eq "1")
    }
    started_at_utc = $StartUtc
    finished_at_utc = $EndUtc
    v0_user_cycle_exit_code = [int]$V0Exit
    system_open_with = [ordered]@{
        state = $ResolverState
        exit_code = [int]$ResolverExit
        provider = "com.android.externalstorage.documents"
        supplied_mime = $ResolverMime
        chaptera_offered = $ResolverChapteraOffered
        unqualified_view_intent = $ResolverUnqualifiedIntent
        useful_page_visible = $ResolverUsefulPageVisible
    }
    physical_perf_exit_code = [int]$PerfExit
    wifi_observation = $WifiObservation
    mobile_data_observation = $DataObservation
    contains_device_serial = $false
    contains_android_id = $false
    internet_permission_requested = $false
    contains_document_bytes = $false
    contains_recovered_document_text = $false
}

$receiptObject | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 $Receipt
Write-Host "Receipt: $Receipt"

if ($V0Exit -ne 0) {
    throw "PHYSICAL_V0_FAILED:$V0Exit"
}
if ($ResolverExit -ne 0) {
    throw "PHYSICAL_OPEN_WITH_FAILED:$ResolverState:mime=$ResolverMime"
}
if ($PerfExit -ne 0) {
    throw "PHYSICAL_PERF_FAILED:$PerfExit"
}

Write-Host "Physical V0, real system Open with resolver, and performance receipt passed."
