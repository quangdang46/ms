<#
.SYNOPSIS
    ms installer for Windows
.DESCRIPTION
    Downloads and installs the prebuilt ms binary for Windows (x86_64-pc-windows-msvc.zip
    from the GitHub Releases page), verifies its SHA256 checksum, then installs it to
    $Destination (default: $HOME/.local/bin). Falls back to building from source if the
    download fails.

    Usage (PowerShell):
        irm https://raw.githubusercontent.com/quangdang46/ms/main/install.ps1 | iex

    Options:
        -Destination  Installation directory (default: "$HOME\.local\bin")
        -Version      Version to install (default: "latest")
        -Verify       Skip checksum verification (default: $true)
        -EasyMode     Auto-add install directory to PATH in PowerShell profile (default: $false)
        -FromSource   Build from source via cargo (default: $false)
        -Uninstall    Remove installed binary (default: $false)
        -NoMcp        Skip MCP provider auto-configuration (default: $false)

    Examples:
        .\install.ps1                           # install latest
        .\install.ps1 -Version "v0.1.1"         # specific version
        .\install.ps1 -EasyMode                  # auto-configure PATH
        .\install.ps1 -Uninstall                 # remove binary
#>
param(
    [string]$Destination = "$HOME\.local\bin",
    [string]$Version      = "latest",
    [switch] $Verify       = $true,
    [switch] $EasyMode     = $false,
    [switch] $FromSource   = $false,
    [switch] $Uninstall    = $false,
    [switch] $NoMcp        = $false,
    [switch] $Quiet        = $false
)

$ErrorActionPreference = "Stop"
$BINARY_NAME = "ms"
$BINARY_FILE = "ms.exe"
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
        Write-Host "$Color[ms]$(Get-CdpColor NC) $Message"
    }
}
function Write-LogWarn($Message) {
    $Color = Get-CdpColor "Yellow"
    [Console]::Error.WriteLine("$Color[ms] WARN: $Message$(Get-CdpColor NC)")
}
function Write-LogError($Message) {
    $Color = Get-CdpColor "Red"
    [Console]::Error.WriteLine("$Color[ms] ERROR: $Message$(Get-CdpColor NC)")
}
function Write-Die($Message) {
    Write-LogError $Message
    exit 1
}

$TempDir = $null
function Clear-Temp {
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
        Write-LogInfo "Not found at $Target -- nothing to uninstall"
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

# Pre-compiled regex patterns to avoid []-interpretation issues in PowerShell
$VERSION_PATTERN = "^[vV]?\d+\.\d+\.\d+(-[\w.]+)?$"
$CHECKSUM_LINE_PATTERN = "^\s*([a-fA-F0-9]+)\s+"
$VERSION_URL_PATTERN = "tag/([^/]+)$"

# === Version resolution ===
function Resolve-Version {
    if ($Version -ne "latest") {
        if ($Version -notmatch $VERSION_PATTERN) {
            Write-Die "Invalid version format: $Version (expected vX.Y.Z or X.Y.Z)"
        }
        if ($Version -notmatch "^v") { $Version = "v$Version" }
        return $Version
    }

    Write-LogInfo "Fetching latest version..."
    try {
        $EffectiveUrl = (Invoke-WebRequest -Uri "https://github.com/${REPO}/releases/latest" `
            -MaximumRedirection 0 -ErrorAction SilentlyContinue).Headers.Location
        if ($EffectiveUrl -match $VERSION_URL_PATTERN) {
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
function Expand-ArchiveSafe {
    param([string]$ArchivePath, [string]$DestinationPath)
    try {
        Expand-Archive -LiteralPath $ArchivePath -DestinationPath $DestinationPath -Force
    } catch {
        Write-LogWarn "Expand-Archive failed, trying Shell.Application fallback..."
        $Shell = New-Object -ComObject Shell.Application
        $Zip = $Shell.Namespace((Resolve-Path $ArchivePath).Path)
        $Shell.Namespace((Resolve-Path $DestinationPath).Path).CopyHere($Zip.Items(), 0x10)
    }
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
            if ($_ -match "${CHECKSUM_LINE_PATTERN}$([Regex]::Escape($ArchiveName))$") {
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
    if (Test-Path $ExtractDir) { Remove-Item $ExtractDir -Recurse -Force }
    New-Item $ExtractDir -ItemType Directory -Force | Out-Null
    Expand-ArchiveSafe -ArchivePath $ArchivePath -DestinationPath $ExtractDir

    $BinaryPath = Get-ChildItem $ExtractDir -Recurse -File -Filter $BINARY_FILE | Select-Object -First 1
    if (-not $BinaryPath) { Write-Die "Could not find $BINARY_FILE in archive" }
    $BinaryPath = $BinaryPath.FullName

    if (-not (Test-Path $Destination)) {
        New-Item $Destination -ItemType Directory -Force | Out-Null
    }
    $InstallDestDir = Get-Item $Destination
    $TargetPath = Join-Path $InstallDestDir.FullName $BINARY_FILE

    # Atomic: write to temp then move
    $TempTarget = "$TargetPath.tmp"
    Copy-Item $BinaryPath $TempTarget -Force
    Move-Item $TempTarget $TargetPath -Force
    if (-not (Test-Path $TargetPath)) { Write-Die "Install failed" }
    Write-LogInfo "Installed to $TargetPath"
}

function Add-ToPath($Path) {
    # Check if PATH already contains the install directory
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -and ($userPath -like "*${Path}*")) {
        Write-LogInfo "Install directory already on PATH"
        return
    }

    if ($EasyMode) {
        try {
            [Environment]::SetEnvironmentVariable('Path', "${Path};${userPath}", 'User')
            Write-LogWarn "PATH updated (User scope). Restart terminal or log out/in to use ms from anywhere."
        } catch {
            Write-LogWarn "Could not update system PATH. Add manually: `$env:PATH += `";$Path`""
        }
    } else {
        Write-LogInfo "To add ms to PATH, run: `$env:Path += `";$Path`""
        Write-LogInfo "Or re-run with -EasyMode for permanent PATH update"
    }
}

# === Source build fallback ===
function Build-FromSource {
    $HasCargo = Get-Command cargo -EA SilentlyContinue
    if (-not $HasCargo) { Write-Die "cargo not found. Install Rust: https://rustup.rs" }

    Write-LogInfo "Building from source..."
    $CloneDir = Join-Path $TempDir "ms-src"
    if (Test-Path $CloneDir) { Remove-Item $CloneDir -Recurse -Force }
    git clone --depth 1 "https://github.com/$REPO" $CloneDir | Out-Null
    Push-Location $CloneDir
    try {
        $Env:CARGO_TARGET_DIR = Join-Path $TempDir "target"
        cargo build --release
        $Env:CARGO_TARGET_DIR = $null
        $BuiltBin = Join-Path $CloneDir "target\release\$BINARY_NAME.exe"
        if (-not (Test-Path $BuiltBin)) { Write-Die "Source build failed" }
        $TargetPath = Join-Path $Destination $BINARY_NAME
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
                Write-LogWarn "Download failed -- building from source..."
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
# Check for binary (with or without .exe extension)
$MsBinary = $null
if (Test-Path (Join-Path $Destination $BINARY_NAME)) {
    $MsBinary = Join-Path $Destination $BINARY_NAME
} elseif (Test-Path (Join-Path $Destination $BINARY_FILE)) {
    $MsBinary = Join-Path $Destination $BINARY_FILE
}

if ($MsBinary) {
    Write-LogInfo ""
    Write-Host "  $(Get-CdpColor Green)SUCCESS$(Get-CdpColor NC) -- ms $Version installed to $MsBinary"
    Write-Host ""
    Write-Host "  Run 'ms --help' to get started."

    if (-not $NoMcp) {
        Configure-AllMcpProviders $MsBinary
    }
    Write-Host ""
} else {
    Write-LogWarn "Binary installed but not found at $Destination"
    Write-LogWarn "Try re-running with -FromSource"
}

# === MCP Provider Auto-Configuration ===

<#
.SYNOPSIS
    Merge JSON into a file at a specified key path.
.DESCRIPTION
    Adds or updates a JSON value at the given top-level key in a JSON file.
#>
function Merge-JsonIntoFile {
    param([string]$FilePath, [string]$Key, [hashtable]$Value)
    $data = @{}
    if (Test-Path $FilePath) {
        $content = Get-Content -Path $FilePath -Raw -ErrorAction SilentlyContinue
        if ($content) {
            try { $data = $content | ConvertFrom-Json -AsHashtable } catch { $data = @{} }
        }
    }
    if (-not $data.ContainsKey($Key)) { $data[$Key] = @{} }
    foreach ($k in $Value.Keys) { $data[$Key][$k] = $Value[$k] }
    $dir = Split-Path $FilePath -Parent
    if (-not (Test-Path $dir)) { New-Item -Path $dir -ItemType Directory -Force | Out-Null }
    $data | ConvertTo-Json -Depth 10 | Set-Content -Path $FilePath -Encoding UTF8
}

<#
.SYNOPSIS
    Register ms as an MCP server for a single JSON-based provider.
#>
function Register-McpProvider {
    param([string]$ProviderName, [string]$SettingsFile, [string]$JsonKey, [string]$BinaryPath)
    if (-not (Test-Path $BinaryPath)) { return }
    Write-LogInfo "  Configuring MCP for $ProviderName..."
    $mcpEntry = @{
        "ms" = @{
            command = $BinaryPath
            args    = @("mcp", "serve")
            env     = @{}
        }
    }
    Merge-JsonIntoFile -FilePath $SettingsFile -Key $JsonKey -Value $mcpEntry
}

<#
.SYNOPSIS
    Register ms MCP in Codex CLI (TOML format).
#>
function Register-McpCodex {
    param([string]$BinaryPath)
    $configFile = Join-Path $env:USERPROFILE ".codex\config.toml"
    $configDir = Split-Path $configFile -Parent
    if (-not (Test-Path $configDir)) { return }
    Write-LogInfo "  Configuring MCP for Codex CLI..."
    $content = ""
    if (Test-Path $configFile) {
        $content = Get-Content $configFile -Raw
    }
    $serverBlock = @"

[mcp_servers.ms]
type = "stdio"
command = "$BinaryPath"
args = ["mcp", "serve"]
"@
    $content += $serverBlock
    $dir = Split-Path $configFile -Parent
    if (-not (Test-Path $dir)) { New-Item $dir -ItemType Directory -Force | Out-Null }
    Set-Content -Path $configFile -Value $content -Encoding UTF8
}

<#
.SYNOPSIS
    Register ms MCP in OpenCode (uses env as array).
#>
function Register-McpOpenCode {
    param([string]$BinaryPath)
    $settingsFile = Join-Path $env:USERPROFILE ".opencode.json"
    if (-not (Test-Path $settingsFile)) {
        $xdgPath = Join-Path $env:USERPROFILE ".config\opencode\.opencode.json"
        if (Test-Path $xdgPath) { $settingsFile = $xdgPath } else { return }
    }
    Write-LogInfo "  Configuring MCP for OpenCode..."
    $mcpEntry = @{
        "ms" = @{
            type    = "stdio"
            command = $BinaryPath
            args    = @("mcp", "serve")
            env     = @()
        }
    }
    Merge-JsonIntoFile -FilePath $settingsFile -Key "mcpServers" -Value $mcpEntry
}

<#
.SYNOPSIS
    Configure all 10 MCP providers for the ms MCP server.
#>
function Configure-AllMcpProviders {
    param([string]$BinaryPath)

    $mcpEntry = @{
        "ms" = @{
            command = $BinaryPath
            args    = @("mcp", "serve")
            env     = @{}
        }
    }

    Write-LogInfo "Configuring MCP providers for AI coding agents..."

    # 1. Claude Code -- ~/.claude.json (root of home)
    Register-McpProvider -ProviderName "Claude Code" -SettingsFile (Join-Path $env:USERPROFILE ".claude.json") -JsonKey "mcpServers" -BinaryPath $BinaryPath

    # 2. Cursor -- ~/.cursor/mcp.json
    Register-McpProvider -ProviderName "Cursor" -SettingsFile (Join-Path $env:USERPROFILE ".cursor\mcp.json") -JsonKey "mcpServers" -BinaryPath $BinaryPath

    # 3. Cline -- VS Code globalStorage
    $clinePath = Join-Path $env:APPDATA "Code\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json"
    if (Test-Path (Split-Path $clinePath -Parent)) {
        Register-McpProvider -ProviderName "Cline" -SettingsFile $clinePath -JsonKey "mcpServers" -BinaryPath $BinaryPath
    }

    # 4. Windsurf -- ~/.codeium/windsurf/mcp_config.json
    Register-McpProvider -ProviderName "Windsurf" -SettingsFile (Join-Path $env:USERPROFILE ".codeium\windsurf\mcp_config.json") -JsonKey "mcpServers" -BinaryPath $BinaryPath

    # 5. VS Code Copilot -- uses "servers" key
    Register-McpProvider -ProviderName "VS Code Copilot" -SettingsFile (Join-Path $env:USERPROFILE ".vscode\mcp.json") -JsonKey "servers" -BinaryPath $BinaryPath

    # 6. OpenCode -- special env format
    Register-McpOpenCode -BinaryPath $BinaryPath

    # 7. Codex CLI -- TOML
    Register-McpCodex -BinaryPath $BinaryPath

    # 8. Gemini CLI -- ~/.gemini/settings.json
    Register-McpProvider -ProviderName "Gemini CLI" -SettingsFile (Join-Path $env:USERPROFILE ".gemini\settings.json") -JsonKey "mcpServers" -BinaryPath $BinaryPath

    # 9. Amazon Q -- write both paths
    Register-McpProvider -ProviderName "Amazon Q" -SettingsFile (Join-Path $env:USERPROFILE ".aws\amazonq\mcp.json") -JsonKey "mcpServers" -BinaryPath $BinaryPath
    Register-McpProvider -ProviderName "Amazon Q (IDE)" -SettingsFile (Join-Path $env:USERPROFILE ".aws\amazonq\default.json") -JsonKey "mcpServers" -BinaryPath $BinaryPath

    # 10. Warp -- project-scoped .warp/.mcp.json
    if (Test-Path ".warp" -PathType Container -or (Test-Path "Cargo.toml")) {
        Register-McpProvider -ProviderName "Warp" -SettingsFile ".warp\.mcp.json" -JsonKey "mcpServers" -BinaryPath $BinaryPath
    }

    Write-LogInfo "MCP provider configuration complete."
}
