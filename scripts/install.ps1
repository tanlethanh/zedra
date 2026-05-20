# Zedra Host installer for Windows.
#
# Usage:
#   powershell -c "irm https://zedra.dev/install.ps1 | iex"
#
# Optional direct usage after download:
#   .\install.ps1 -Version v0.2.4 -Prefix "$env:LOCALAPPDATA\Programs\zedra\bin"

param(
    [string]$Version = "",
    [string]$Prefix = "",
    [switch]$NoPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Repo = "tanlethanh/zedra"
$Binary = "zedra.exe"

function Get-DefaultPrefix {
    $localAppData = [Environment]::GetFolderPath("LocalApplicationData")
    if ([string]::IsNullOrWhiteSpace($localAppData)) {
        $localAppData = Join-Path $HOME "AppData\Local"
    }
    Join-Path $localAppData "Programs\zedra\bin"
}

function Resolve-InstallPrefix {
    param([string]$RequestedPrefix)

    if (-not [string]::IsNullOrWhiteSpace($RequestedPrefix)) {
        return $RequestedPrefix
    }
    if (-not [string]::IsNullOrWhiteSpace($env:ZEDRA_PREFIX)) {
        return $env:ZEDRA_PREFIX
    }
    Get-DefaultPrefix
}

function Get-ZedraTarget {
    $arch = $env:PROCESSOR_ARCHITEW6432
    if ([string]::IsNullOrWhiteSpace($arch)) {
        $arch = $env:PROCESSOR_ARCHITECTURE
    }

    switch ($arch.ToUpperInvariant()) {
        "AMD64" { return "x86_64-pc-windows-msvc" }
        default {
            throw "pre-built Windows binaries are not available for architecture: $arch. Build from source with: cargo install --git https://github.com/$Repo zedra-host"
        }
    }
}

function Resolve-ZedraVersion {
    param([string]$RequestedVersion)

    if (-not [string]::IsNullOrWhiteSpace($RequestedVersion)) {
        return $RequestedVersion
    }

    try {
        $headers = @{ "User-Agent" = "zedra-installer" }
        $release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$Repo/releases/latest"
        if ([string]::IsNullOrWhiteSpace($release.tag_name)) {
            throw "missing tag_name"
        }
        return $release.tag_name
    } catch {
        throw "failed to resolve latest version. Specify one with -Version. $($_.Exception.Message)"
    }
}

function Verify-ZedraChecksum {
    param(
        [string]$ArchivePath,
        [string]$ChecksumUrl
    )

    try {
        $response = Invoke-WebRequest -Uri $ChecksumUrl -UseBasicParsing
    } catch {
        Write-Host "  (checksum file not available, skipping verification)"
        return
    }

    $content = $response.Content
    if ($content -is [byte[]]) {
        # Windows PowerShell can expose extensionless GitHub assets as raw bytes.
        $expectedText = [System.Text.Encoding]::UTF8.GetString($content)
    } else {
        $expectedText = [string]$content
    }

    $match = [regex]::Match($expectedText, "(?im)^\s*([a-f0-9]{64})\b")
    if (-not $match.Success) {
        Write-Host "  (checksum file empty, skipping verification)"
        return
    }
    $expectedHash = $match.Groups[1].Value.ToLowerInvariant()

    $actualHash = (Get-FileHash -Algorithm SHA256 -Path $ArchivePath).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "checksum mismatch! expected: $expectedHash actual: $actualHash"
    }

    Write-Host "  Checksum verified."
}

function Invoke-ZedraDownload {
    param(
        [string]$Url,
        [string]$OutFile,
        [string]$Version,
        [string]$Target
    )

    try {
        Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
    } catch {
        $statusCode = $null
        if ($null -ne $_.Exception.Response) {
            $statusCode = [int]$_.Exception.Response.StatusCode
        }

        if ($statusCode -eq 404) {
            throw "release $Version does not include a prebuilt Windows binary for $Target. Try a newer release with Windows assets, or build from source with: cargo install --git https://github.com/$Repo zedra-host"
        }

        throw "failed to download $Url. $($_.Exception.Message)"
    }
}

function Assert-PrefixWritable {
    param([string]$InstallPrefix)

    New-Item -ItemType Directory -Force -Path $InstallPrefix | Out-Null
    $probe = Join-Path $InstallPrefix ".zedra-write-test-$([Guid]::NewGuid())"
    try {
        New-Item -ItemType File -Path $probe -Force | Out-Null
    } catch {
        throw "install directory is not writable: $InstallPrefix. Choose another directory with -Prefix or ZEDRA_PREFIX."
    } finally {
        Remove-Item -Path $probe -Force -ErrorAction SilentlyContinue
    }
}

function Test-PathContains {
    param(
        [string]$PathValue,
        [string]$Entry
    )

    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $false
    }

    $target = [System.IO.Path]::GetFullPath($Entry).TrimEnd("\")
    foreach ($part in ($PathValue -split ";")) {
        if ([string]::IsNullOrWhiteSpace($part)) {
            continue
        }
        try {
            $normalized = [System.IO.Path]::GetFullPath($part).TrimEnd("\")
        } catch {
            $normalized = $part.TrimEnd("\")
        }
        if ([string]::Equals($normalized, $target, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }

    $false
}

function Add-ZedraToPath {
    param([string]$InstallPrefix)

    if (-not (Test-PathContains $env:Path $InstallPrefix)) {
        $env:Path = "$InstallPrefix;$env:Path"
    }

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if ((Test-PathContains $userPath $InstallPrefix) -or (Test-PathContains $machinePath $InstallPrefix)) {
        return
    }

    if ([string]::IsNullOrWhiteSpace($userPath)) {
        [Environment]::SetEnvironmentVariable("Path", $InstallPrefix, "User")
    } else {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallPrefix", "User")
    }

    Write-Host "  Added $InstallPrefix to the user PATH. Open a new terminal to use it everywhere."
}

function Warn-IfShadowed {
    param([string]$InstalledBinary)

    $command = Get-Command "zedra.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $command -or [string]::IsNullOrWhiteSpace($command.Source)) {
        return
    }

    $resolvedInstalled = [System.IO.Path]::GetFullPath($InstalledBinary)
    $resolvedCommand = [System.IO.Path]::GetFullPath($command.Source)
    if (-not [string]::Equals($resolvedInstalled, $resolvedCommand, [System.StringComparison]::OrdinalIgnoreCase)) {
        Write-Warning "zedra.exe on PATH resolves to $resolvedCommand. Move this install directory earlier in PATH or remove the older installation."
    }
}

function Install-Zedra {
    param(
        [string]$Version = "",
        [string]$Prefix = "",
        [switch]$NoPath
    )

    $installPrefix = Resolve-InstallPrefix $Prefix
    $target = Get-ZedraTarget
    $resolvedVersion = Resolve-ZedraVersion $Version

    Write-Host "Installing zedra $resolvedVersion for $target..."

    $baseUrl = "https://github.com/$Repo/releases/download/$resolvedVersion"
    $archiveName = "zedra-$target.tar.gz"
    $archiveUrl = "$baseUrl/$archiveName"
    $checksumUrl = "$archiveUrl.sha256"
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "zedra-install-$([Guid]::NewGuid())"

    New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
    try {
        $archivePath = Join-Path $tmpDir $archiveName
        Write-Host "  Downloading $archiveUrl..."
        Invoke-ZedraDownload -Url $archiveUrl -OutFile $archivePath -Version $resolvedVersion -Target $target

        Verify-ZedraChecksum -ArchivePath $archivePath -ChecksumUrl $checksumUrl

        Write-Host "  Extracting..."
        if (-not (Get-Command "tar.exe" -ErrorAction SilentlyContinue)) {
            throw "tar.exe was not found. Install a current Windows 10/11 build or extract $archiveName manually."
        }
        & tar.exe -xzf $archivePath -C $tmpDir
        if ($LASTEXITCODE -ne 0) {
            throw "failed to extract $archiveName"
        }

        $extracted = Join-Path $tmpDir $Binary
        if (-not (Test-Path $extracted)) {
            throw "archive did not contain $Binary"
        }

        Assert-PrefixWritable $installPrefix
        $installedBinary = Join-Path $installPrefix $Binary

        Write-Host "  Installing to $installedBinary..."
        Copy-Item -Force $extracted $installedBinary

        if (-not $NoPath) {
            Add-ZedraToPath $installPrefix
        }

        Write-Host ""
        Write-Host "Installed zedra to $installedBinary"
        Warn-IfShadowed $installedBinary
        Write-Host "Run 'zedra --help' to get started."
    } finally {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Zedra -Version $Version -Prefix $Prefix -NoPath:$NoPath
