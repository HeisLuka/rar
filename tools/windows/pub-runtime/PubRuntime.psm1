Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Write-PubJson {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Value,
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $parent = Split-Path -Parent $Path
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }

    $Value | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Format-PubHResult {
    param(
        [Parameter(Mandatory = $true)]
        [int]$HResult
    )

    return ('0x{0:X8}' -f ($HResult -band 0xFFFFFFFFL))
}

function Get-PubFileRecord {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $item = Get-Item -LiteralPath $resolved
    if ($item.PSIsContainer) {
        throw "Ожидался файл, но получен каталог: $resolved"
    }

    $hash = Get-FileHash -LiteralPath $resolved -Algorithm SHA256
    return [ordered]@{
        path = $resolved
        name = $item.Name
        size = [int64]$item.Length
        sha256 = $hash.Hash.ToLowerInvariant()
    }
}

function Get-PubSafeValue {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Getter,
        [Parameter(Mandatory = $true)]
        [string]$Member
    )

    try {
        return [ordered]@{
            state = "value"
            member = $Member
            value = & $Getter
        }
    }
    catch {
        $hresult = $null
        if ($_.Exception.HResult) {
            $hresult = Format-PubHResult ([int]$_.Exception.HResult)
        }

        return [ordered]@{
            state = "error"
            member = $Member
            hresult = $hresult
            message = $_.Exception.Message
        }
    }
}

function New-PubPublisherApplication {
    param(
        [switch]$Visible
    )

    $application = New-Object -ComObject Publisher.Application
    if ($Visible) {
        try {
            $application.ActiveWindow.Visible = $true
        }
        catch {
            # До открытия документа ActiveWindow может отсутствовать.
        }
    }

    return $application
}

function Close-PubPublisherApplication {
    param(
        [Parameter(Mandatory = $false)]
        $Application
    )

    if ($null -eq $Application) {
        return
    }

    try {
        $Application.Quit()
    }
    catch {
        Write-Warning "Publisher.Quit завершился ошибкой: $($_.Exception.Message)"
    }

    try {
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($Application)
    }
    catch {
        # Освобождение RCW — best effort; provenance результата важнее cleanup-ошибки.
    }

    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}

function Get-PubPublisherIdentity {
    param(
        [switch]$Visible
    )

    $application = $null
    try {
        $application = New-PubPublisherApplication -Visible:$Visible
        return [ordered]@{
            available = $true
            version = Get-PubSafeValue { [string]$application.Version } "Application.Version"
            build = Get-PubSafeValue { [string]$application.Build } "Application.Build"
            name = Get-PubSafeValue { [string]$application.Name } "Application.Name"
            path = Get-PubSafeValue { [string]$application.Path } "Application.Path"
        }
    }
    catch {
        $hresult = $null
        if ($_.Exception.HResult) {
            $hresult = Format-PubHResult ([int]$_.Exception.HResult)
        }

        return [ordered]@{
            available = $false
            hresult = $hresult
            message = $_.Exception.Message
        }
    }
    finally {
        Close-PubPublisherApplication $application
    }
}

function Get-PubEnvironmentManifest {
    param(
        [string]$SnapshotId = "",
        [switch]$RequirePublisher,
        [switch]$Visible
    )

    $publisher = Get-PubPublisherIdentity -Visible:$Visible
    if ($RequirePublisher -and -not $publisher.available) {
        throw "Microsoft Publisher COM automation недоступна: $($publisher.message)"
    }

    $culture = [System.Globalization.CultureInfo]::CurrentCulture
    $uiCulture = [System.Globalization.CultureInfo]::CurrentUICulture

    $timezone = $null
    try {
        $timezone = [System.TimeZoneInfo]::Local.Id
    }
    catch {
        $timezone = $null
    }

    $defaultPrinter = $null
    try {
        $defaultPrinter = (Get-CimInstance Win32_Printer -Filter "Default=True" -ErrorAction Stop |
            Select-Object -First 1 -ExpandProperty Name)
    }
    catch {
        $defaultPrinter = $null
    }

    return [ordered]@{
        schema = "pub-runtime/environment/v1"
        captured_at = [DateTimeOffset]::Now.ToString("o")
        snapshot_id = $SnapshotId
        os = [ordered]@{
            version = [System.Environment]::OSVersion.VersionString
            is_64_bit_os = [System.Environment]::Is64BitOperatingSystem
            is_64_bit_process = [System.Environment]::Is64BitProcess
        }
        powershell = [ordered]@{
            version = $PSVersionTable.PSVersion.ToString()
            edition = if ($PSVersionTable.PSEdition) { [string]$PSVersionTable.PSEdition } else { "Desktop" }
        }
        locale = [ordered]@{
            culture = $culture.Name
            ui_culture = $uiCulture.Name
            ansi_code_page = $culture.TextInfo.ANSICodePage
        }
        timezone = $timezone
        default_printer = $defaultPrinter
        machine = [System.Environment]::MachineName
        publisher = $publisher
    }
}

function Copy-PubBoundFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Source,
        [Parameter(Mandatory = $true)]
        [string]$Destination
    )

    $before = Get-PubFileRecord $Source
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    Copy-Item -LiteralPath $before.path -Destination $Destination

    $after = Get-PubFileRecord $Destination
    if ($before.sha256 -ne $after.sha256 -or $before.size -ne $after.size) {
        throw "Binding файла нарушен при копировании: $($before.path)"
    }

    return [ordered]@{
        source = $before
        bound_copy = $after
    }
}

function Get-PubDirectoryHashes {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )

    $rootPath = (Resolve-Path -LiteralPath $Root).Path
    $records = @()

    Get-ChildItem -LiteralPath $rootPath -Recurse -File |
        Sort-Object FullName |
        ForEach-Object {
            $hash = Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256
            $relative = $_.FullName.Substring($rootPath.Length).TrimStart('\', '/')
            $records += [ordered]@{
                path = $relative.Replace('\', '/')
                size = [int64]$_.Length
                sha256 = $hash.Hash.ToLowerInvariant()
            }
        }

    return $records
}

function Write-PubHashList {
    param(
        [Parameter(Mandatory = $true)]
        [array]$Records,
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $lines = @()
    foreach ($record in $Records) {
        $lines += "$($record.sha256)  $($record.path)"
    }
    $lines | Set-Content -LiteralPath $Path -Encoding ascii
}

Export-ModuleMember -Function @(
    "Write-PubJson",
    "Format-PubHResult",
    "Get-PubFileRecord",
    "Get-PubSafeValue",
    "New-PubPublisherApplication",
    "Close-PubPublisherApplication",
    "Get-PubPublisherIdentity",
    "Get-PubEnvironmentManifest",
    "Copy-PubBoundFile",
    "Get-PubDirectoryHashes",
    "Write-PubHashList"
)
