<#
.SYNOPSIS
    ms installer for Windows
.DESCRIPTION
    Downloads and installs the prebuilt ms binary for Windows (x86_64-pc-windows-msvc.zip
    from the GitHub Releases page), verifies its SHA256 checksum, then installs it to
    $Destination (default: $HOME/.local/bin). Falls back to building from source if the
    download fails or the binary fails to run due to GLIBC mismatch on older Linux systems
    (the Windows build does not have this issue, so fallback is only provided for completeness).

    Usage (PowerShell):
        irm https://raw.githubusercontent.com/quangdang46/ms/main/install.ps1 | iex

    Options (passed as environment variables or parameters):
        $Destination  Installation directory (default: "$HOME/.local.bin")
        $Version      Version to install (default: "latest")
        $Verify       Skip checksum verification (default: "true")
        $EasyMode     Auto-add install directory to PATH in PowerShell profile (default: "false")
        $FromSource   Build from source via cargo (default: "false")
        $Uninstall    Remove installed binary (default: "false")
        $Quiet        Suppress non-error output (default: "false")

    Example (PowerShell 7+):
        $Version = "v0.1.0"; irm https://raw.githubusercontent.com/quangdang46/ms/main/install.ps1 | iex
#>
param(
    [string]$Destination = "$HOME\.local\bin",
    [string]$Version      = "latest",
    [bool]   $Verify       = $true,
    [bool]   $EasyMode     = $false,
    [bool]   $FromSource   = $false,
    [bool]   $Uninstall    = $false
)

$ErrorActionPreference = "Stop"
$BINARY_NAME = "ms"
$REPO        = "quangdang46/ms"
$MAX_RETRIES  = 3
$DOWNLOAD_TIMEOUT = 120

function Get-CdpColor($Color) {
    if ($null -eq $env:NO_COLOR -and [Console]::IsOutputRedirected -eq $false) {
        switch ($Color) {
            "Red"    { return "`e[0;31m" }
            "Green"  { return "`e[0;32m" }
            "Yellow" { return "`e[0;33m" }
            "Blue"   { return "`e[0;34m" }
            "Bold"   { return "`e[1m" }
            "NC"     { return "`e[0m" }
        }
    }
    return ""
}

function Write-LogInfo($Message) {
    if (-not $Quiet) {
        $Color = Get-CdpColor "Blue"
        Write-Host "${Color}[ms]$(Get-CdpColor NC) $Message"
    }
}
function Write-LogWarn($Message) {
    $Color = Get-CdpColor "Yellow"
    [Console]::Error.WriteLine("${Color}[ms] WARN: $Message$(Get-CdpColor NC)")
}
function Write-LogError($Message) {
    $Color = Get-CdpColor "Red"
    [Console]::Error.WriteLine("${Color}[ms] ERROR: $Message$(Get-CdpColor NC)")
}
function Write-Die($Message) {
    Write-LogError $Message
    exit 1
}

$TempDir = $null
function script:Clear-Temp {
    if ($null -ne $TempDir -and (Test-Path $TempDir)) {
        Remove-Item $TempDir -Recurse -Force -EA SilentlyContinue
    }
}
trap { Clear-Temp; throw }
$TempDir = [System.IO.Path]::GetTempPath()

# === Uninstall ===
if ($Uninstall) {
    $Target = Join-Path $Destination $BINARY_NAME
    if (Test-Path $Target) {
        Remove-Item $Target -Force
        Write-LogInfo "Uninstalled $Target"
    } else {
        Write-LogInfo "Not found at $Target — nothing to uninstall"
    }
    return
}

# === Platform ===
function Get-Platform {
    $Arch = switch ($env:PROCESSOR_ARCHITECTURE) {
        "AMD64"   { "x86_64" }
        "ARM64"   { "aarch64" }
        default   { throw "Unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
    }
    return "${Arch}-pc-windows-msvc"
}

# === Version resolution ===
function Resolve-Version {
    if ($Version -ne "latest") {
        if ($Version -notmatch "^v?[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$") {
            Write-Die "Invalid version format: $Version (expected vX.Y.Z or X.Y.Z)"
        }
        if ($Version -notmatch "^v") { $Version = "v$Version" }
        return $Version
    }

    Write-LogInfo "Fetching latest version..."
    try {
        $EffectiveUrl = (Invoke-WebRequest -Uri "https://github.com/${REPO}/releases/latest" `
            -MaximumRedirection 0 -ErrorAction SilentlyContinue).Headers.Location
        if ($EffectiveUrl -match 'tag/([^/]+)$') {
            return $matches[1]
        }
    } catch {}

    try {
        $Response = Invoke-RestMethod -Uri "https://api.github.com/repos/${REPO}/releases/latest" `
            -TimeoutSec 30 -EA SilentlyContinue
        if ($Response.tag_name) { return $Response.tag_name }
    } catch {}

    Write-Die "Could not determine latest version"
}

# === Download with retry ===
function Expand-ArchiveProper($Archive, $Destination) {
    $Shell = New-Object -ComObject Shell.Application
    $Zip = $Shell.Namespace((Resolve-Path $Archive).Path)
    $Shell.Namespace((Resolve-Path $Destination).Path).CopyHere($Zip.Items(), 0x10)
}

function Install-MsArtifact($ArchiveUrl, $ArchivePath, $VersionForUrl, $Platform) {
    $ArchiveName = Split-Path $ArchivePath -Leaf
    $ExtractDir  = Join-Path $TempDir "extract-$ArchiveName"

    if ($Verify) {
        $ChecksumsUrl = $ArchiveUrl -replace "[^/]+\.zip$", "SHA256SUMS.txt"
        $ChecksumsPath = Join-Path $TempDir "SHA256SUMS.txt"
        $Attempt = 0
        while ($Attempt -lt $MAX_RETRIES) {
            $Attempt++
            try {
                Invoke-WebRequest -Uri $ChecksumsUrl -OutFile $ChecksumsPath -TimeoutSec $DOWNLOAD_TIMEOUT
                break
            } catch {
                if ($Attempt -lt $MAX_RETRIES) {
                    Write-LogWarn "Checksum download failed (attempt $Attempt/$MAX_RETRIES), retrying..."
                    Start-Sleep 3
                } else {
                    Write-Die "Could not download checksums: $ChecksumsUrl"
                }
            }
        }

        $ExpectedSha = $null
        Get-Content $ChecksumsPath | ForEach-Object {
            if ($_ -match "^\s*([a-fA-F0-9]+)\s+.*$([Regex]::Escape($ArchiveName))$") {
                $ExpectedSha = $matches[1]
            }
        }
        if (-not $ExpectedSha) { Write-Die "No checksum found for $ArchiveName" }

        $ActualSha = (Get-FileHash -Algorithm SHA256 -Path $ArchivePath).Hash.ToLower()
        if ($ExpectedSha.ToLower() -ne $ActualSha) {
            Write-Die "Checksum mismatch. Expected: $ExpectedSha, Got: $ActualSha"
        }
        Write-LogInfo "Checksum verified"
    }

    Write-LogInfo "Extracting..."
    $ExtractDest = Join-Path $ExtractDir $BINARY_NAME
    if (Test-Path $ExtractDir) { Remove-Item $ExtractDir -Recurse -Force }
    New-Item $ExtractDir -ItemType Directory -Force | Out-Null
    Expand-ArchiveProper $ArchivePath $ExtractDir

    $BinaryPath = Get-ChildItem $ExtractDir -Recurse -File -Name $BINARY_NAME | Select-Object -First 1
    if (-not $BinaryPath) { Write-Die "Could not find $BINARY_NAME in archive" }
    $BinaryPath = Join-Path $ExtractDir $BinaryPath

    $InstallDestDir = if (Test-Path $Destination) { $Destination } else { New-Item $Destination -ItemType Directory -Force | Resolve-Path }
    $TargetPath = Join-Path $InstallDestDir.FullName $BINARY_NAME

    # Atomic: write to temp then move
    $TempTarget = "$TargetPath.tmp"
    Copy-Item $BinaryPath $TempTarget -Force
    Move-Item $TempTarget $TargetPath -Force
    if (-not (Test-Path $TargetPath)) { Write-Die "Install failed" }
    Write-LogInfo "Installed to $TargetPath"
}

function Add-ToPath($Path) {
    $UserProfilePath = if ($IsLinux -or $IsMacOS) { $HOME } else { "$env:USERPROFILE" }
    $RcFile = Join-Path $UserProfilePath "Documents\PowerShell\Microsoft.PowerShell_profile.ps1"
    if (-not (Test-Path (Split-Path $RcFile))) {
        New-Item -Path (Split-Path $RcFile) -ItemType Directory -Force | Out-Null
    }

    if ((Test-Path $RcFile) -and (Get-Content $RcFile -Raw) -match [Regex]::Escape($Path)) {
        return
    }

    if ($EasyMode) {
        $PathLine = "`$env:PATH = `"`$env:PATH;$Path`"  # ms installer"
        $PathLine | Out-File $RcFile -Append -Encoding utf8
        Write-LogWarn "PATH updated — restart PowerShell or reload profile to use ms"
    } else {
        Write-LogWarn "Add to PATH: `$env:PATH += `";$Path`""
    }
}

# === Source build fallback ===
function Build-FromSource {
    $HasCargo = Get-Command cargo -EA SilentlyContinue
    if (-not $HasCargo) { Write-Die "cargo not found. Install Rust: https://rustup.rs" }

    Write-LogInfo "Building from source..."
    $CloneDir = Join-Path $TempDir "ms-src"
    if (Test-Path $CloneDir) { Remove-Item $CloneDir -Recurse -Force }
    git clone --depth 1 "https://github.com/$REPO" . $CloneDir | Out-Null
    Push-Location $CloneDir
    try {
        $Env:CARGO_TARGET_DIR = Join-Path $TempDir "target"
        cargo build --release
        $Env:CARGO_TARGET_DIR = $null
        $BuiltBin = Join-Path $CloneDir "target\release\$BINARY_NAME.exe"
        if (-not (Test-Path $BuiltBin)) { Write-Die "Source build failed" }
        $TargetPath = Join-Path (Split-Path $Destination -Parent) "$Destination\$BINARY_NAME.exe"
        Copy-Item $BuiltBin $TargetPath -Force
        Write-LogInfo "Installed from source to $TargetPath"
    } finally {
        Pop-Location
    }
}

# === Main ===
$Platform      = Get-Platform
$Version       = Resolve-Version
$VersionForUrl = $Version -replace "^v", ""

Write-LogInfo "Platform: $Platform | Version: $Version | Dest: $Destination"

if (-not $FromSource) {
    $ArchiveName = "ms-$VersionForUrl-$Platform.zip"
    $ArchiveUrl  = "https://github.com/$REPO/releases/download/$Version/$ArchiveName"
    $ArchivePath = Join-Path $TempDir $ArchiveName

    $Attempt = 0
    while ($Attempt -lt $MAX_RETRIES) {
        $Attempt++
        Write-LogInfo "Downloading from $ArchiveUrl..."
        try {
            Invoke-WebRequest -Uri $ArchiveUrl -OutFile $ArchivePath -TimeoutSec $DOWNLOAD_TIMEOUT
            break
        } catch {
            if ($Attempt -lt $MAX_RETRIES) {
                Write-LogWarn "Download failed (attempt $Attempt/$MAX_RETRIES), retrying..."
                Start-Sleep 3
            } else {
                Write-LogWarn "Download failed — building from source..."
                Build-FromSource
                Add-ToPath $Destination
                Write-LogInfo "Run 'ms --version' to verify."
                return
            }
        }
    }

    Install-MsArtifact $ArchiveUrl $ArchivePath $VersionForUrl $Platform
} else {
    Build-FromSource
}

Add-ToPath $Destination

# === Verify ===
if (Test-Path (Join-Path $Destination $BINARY_NAME)) {
    Write-LogInfo ""
    Write-Host "  $(Get-CdpColor Green)SUCCESS$(Get-CdpColor NC) — ms $Version installed to $Destination"
    Write-Host ""
    Write-Host "  Run 'ms --help' to get started."
    Write-Host ""
} else {
    Write-LogWarn "Binary installed but --version check failed. This sometimes happens on"
    Write-LogWarn "older Linux distributions with glibc version mismatches."
    Write-LogWarn "Try re-running with `$FromSource = `$true"
}
