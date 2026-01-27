# Install bossa from GitHub releases (PowerShell)
# Usage: irm https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.ps1 | iex
#
# Or with options:
#   $env:BOSSA_VERSION = "v0.1.0"; irm ... | iex
#   $env:BOSSA_DIR = "C:\Tools\bossa"; irm ... | iex
#
# Environment variables:
#   BOSSA_VERSION  - Version to install (default: latest)
#   BOSSA_DIR      - Installation directory (default: $env:LOCALAPPDATA\Programs\bossa)
#   BOSSA_REPO     - GitHub repository (default: albertocavalcante/bossa)

#Requires -Version 5.1

$ErrorActionPreference = 'Stop'

$Repo = if ($env:BOSSA_REPO) { $env:BOSSA_REPO } else { "albertocavalcante/bossa" }
$InstallDir = if ($env:BOSSA_DIR) { $env:BOSSA_DIR } else { "$env:LOCALAPPDATA\Programs\bossa" }
$Version = $env:BOSSA_VERSION
$BinaryName = "bossa"

function Write-Info { param($Message) Write-Host "==> " -ForegroundColor Blue -NoNewline; Write-Host $Message }
function Write-Success { param($Message) Write-Host "==> " -ForegroundColor Green -NoNewline; Write-Host $Message }
function Write-Warn { param($Message) Write-Host "warning: " -ForegroundColor Yellow -NoNewline; Write-Host $Message }
function Write-Err { param($Message) Write-Host "error: " -ForegroundColor Red -NoNewline; Write-Host $Message; exit 1 }

function Get-Platform {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture

    switch ($arch) {
        "X64" { return "windows-amd64" }
        "Arm64" { return "windows-arm64" }
        default { Write-Err "Unsupported architecture: $arch" }
    }
}

function Get-LatestVersion {
    $url = "https://api.github.com/repos/$Repo/releases/latest"

    try {
        $response = Invoke-RestMethod -Uri $url -Headers @{ "User-Agent" = "bossa-installer" }
        return $response.tag_name
    }
    catch {
        Write-Err "Failed to fetch latest version: $_"
    }
}

function Get-Checksum {
    param($File)
    return (Get-FileHash -Path $File -Algorithm SHA256).Hash.ToLower()
}

function Test-Checksum {
    param($File, $ChecksumFile)

    if (-not (Test-Path $ChecksumFile)) {
        Write-Warn "Checksum file not found, skipping verification"
        return $true
    }

    $expected = (Get-Content $ChecksumFile -Raw).Trim().Split()[0].ToLower()
    $actual = Get-Checksum $File

    if ($expected -ne $actual) {
        Write-Err "Checksum mismatch!`nExpected: $expected`nActual:   $actual"
    }

    Write-Success "Checksum verified"
    return $true
}

function Main {
    Write-Host ""
    Write-Host "bossa installer" -ForegroundColor White
    Write-Host ""

    # Detect platform
    $platform = Get-Platform
    Write-Info "Detected platform: $platform"

    # Get version
    if (-not $Version) {
        Write-Info "Fetching latest version..."
        $script:Version = Get-LatestVersion
        if (-not $Version) {
            Write-Err "Failed to determine latest version"
        }
    }
    Write-Info "Installing version: $Version"

    # Asset names
    $assetName = "$BinaryName-$platform.zip"
    $checksumName = "$assetName.sha256"

    # Download URLs
    $baseUrl = "https://github.com/$Repo/releases/download/$Version"
    $assetUrl = "$baseUrl/$assetName"
    $checksumUrl = "$baseUrl/$checksumName"

    # Create temp directory
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        # Download asset
        $assetPath = Join-Path $tempDir $assetName
        Write-Info "Downloading $assetName..."
        Invoke-WebRequest -Uri $assetUrl -OutFile $assetPath -UseBasicParsing

        # Download and verify checksum
        $checksumPath = Join-Path $tempDir $checksumName
        Write-Info "Downloading checksum..."
        try {
            Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath -UseBasicParsing -ErrorAction SilentlyContinue
        }
        catch {
            # Checksum file may not exist
        }
        Test-Checksum $assetPath $checksumPath

        # Extract
        Write-Info "Extracting..."
        Expand-Archive -Path $assetPath -DestinationPath $tempDir -Force

        # Install
        Write-Info "Installing to $InstallDir..."
        if (-not (Test-Path $InstallDir)) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        }

        $binaryPath = Join-Path $tempDir "$BinaryName.exe"
        $destPath = Join-Path $InstallDir "$BinaryName.exe"

        Copy-Item -Path $binaryPath -Destination $destPath -Force

        Write-Success "Installed bossa to $destPath"

        # Check PATH
        $userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
        if ($userPath -notlike "*$InstallDir*") {
            Write-Host ""
            Write-Warn "$InstallDir is not in your PATH"
            Write-Host ""
            Write-Host "Add it to your PATH by running:"
            Write-Host ""
            Write-Host "  # PowerShell (permanent)" -ForegroundColor Cyan
            Write-Host '  $path = [Environment]::GetEnvironmentVariable("PATH", "User")'
            Write-Host "  [Environment]::SetEnvironmentVariable(`"PATH`", `"`$path;$InstallDir`", `"User`")"
            Write-Host ""
            Write-Host "  # Or for current session only" -ForegroundColor Cyan
            Write-Host "  `$env:PATH += `";$InstallDir`""
            Write-Host ""
        }

        # Show version
        Write-Host ""
        if (Test-Path $destPath) {
            & $destPath --version
        }

        Write-Host ""
        Write-Success "Installation complete!"
        Write-Host ""
        Write-Host "Get started:"
        Write-Host "  bossa --help"
        Write-Host "  bossa status"
        Write-Host ""
    }
    finally {
        # Cleanup
        if (Test-Path $tempDir) {
            Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Main
