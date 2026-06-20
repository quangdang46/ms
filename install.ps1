#Requires -Version 5.1
<#
.SYNOPSIS
    ms installer for Windows — downloads the right binary from GitHub Releases
    and optionally registers ms as an MCP server with detected AI assistants.
.DESCRIPTION
    Pipe usage (no parameters — uses defaults or env-var overrides):
        irm https://raw.githubusercontent.com/quangdang46/ms/main/install.ps1 | iex

    Direct usage (supports all parameters):
        iwr https://raw.githubusercontent.com/quangdang46/ms/main/install.ps1 -OutFile install.ps1
        .\install.ps1 -Version v0.3.0 -EasyMode

    Env-var fallbacks (for the piped form):
        $env:MS_VERSION          - pin a specific release tag
        $env:MS_INSTALL_DIR      - override install directory
        $env:MS_PATH_SCOPE       - User | Profile | None
        $env:MS_NO_MCP           - set to '1' to skip MCP registration
        $env:MS_MCP_ONLY         - set to '1' for MCP-only (skip binary install)
        $env:MS_MCP_PROVIDERS    - comma-separated list (default: all detected)
        $env:MS_MCP_NAME         - server name (default: ms)

.PARAMETER Version
    Release tag to install (e.g. 'v0.3.0'). Default: latest.
.PARAMETER InstallDir
    Target directory. Default: $env:LOCALAPPDATA\ms\bin.
.PARAMETER PathScope
    How to persist PATH: 'User' (default), 'Profile' (append to $PROFILE), 'None'.
.PARAMETER EasyMode
    Persist PATH and verify after install.
.PARAMETER Verify
    Run ms --version after install as a self-test.
.PARAMETER Uninstall
    Remove the ms binary and PATH entries.
.PARAMETER NoMcp
    Skip MCP registration.
.PARAMETER McpOnly
    Skip binary install; only register MCP (assumes ms is on PATH or in -InstallDir).
.PARAMETER McpProviders
    Comma-separated list of MCP providers to register with. Default: all detected.
.PARAMETER McpName
    Server name written into MCP configs. Default: ms.
.PARAMETER McpDryRun
    Print MCP config writes without touching files.
.PARAMETER McpUninstall
    Remove the ms MCP entry from every provider config.
#>
param(
    [string]$Version      = $env:MS_VERSION,
    [string]$InstallDir   = $env:MS_INSTALL_DIR,
    [ValidateSet('User', 'Profile', 'None', '')]
    [string]$PathScope,
    [switch]$EasyMode,
    [switch]$Verify,
    [switch]$Uninstall,
    [switch]$NoMcp,
    [switch]$McpOnly,
    [string]$McpProviders = $env:MS_MCP_PROVIDERS,
    [string]$McpName      = $env:MS_MCP_NAME,
    [switch]$McpDryRun,
    [switch]$McpUninstall
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

# === Defaults ===
$Owner      = 'quangdang46'
$Repo       = 'ms'
$BinaryName = 'ms'
if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA 'ms\bin' }
if (-not $PathScope) {
    $PathScope = if ($env:MS_PATH_SCOPE) { $env:MS_PATH_SCOPE }
                 elseif ($EasyMode)       { 'User' }
                 else                     { 'User' }
}
if (-not $McpName) { $McpName = 'ms' }
if (-not $McpProviders) { $McpProviders = 'all' }

# === Logging ===
function Write-Info    { param($m) Write-Host "[$BinaryName] $m" }
function Write-Success { param($m) Write-Host "[OK] $m" -ForegroundColor Green }
function Write-Warn    { param($m) Write-Host "[$BinaryName] WARN: $m" -ForegroundColor Yellow }

function Invoke-Quiet {
    param([Parameter(Mandatory)][scriptblock]$Block)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    try { & $Block 2>&1 | Out-Null } finally { $ErrorActionPreference = $prev }
    return $LASTEXITCODE
}

# === Platform detection ===
function Get-Target {
    $arch = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').PROCESSOR_ARCHITECTURE
    switch ($arch) {
        'AMD64' { return 'x86_64-pc-windows-msvc' }
        'ARM64' { return 'aarch64-pc-windows-msvc' }
        default { throw "Unsupported architecture: $arch" }
    }
}

# === Version resolution ===
function Resolve-LatestVersion {
    $headers = @{ 'User-Agent' = 'ms-installer' }
    if ($env:GITHUB_TOKEN) { $headers['Authorization'] = "Bearer $env:GITHUB_TOKEN" }

    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Owner/$Repo/releases/latest" -Headers $headers
        if ($release.tag_name -match '^v[\d]') { return $release.tag_name }
    } catch {}

    try {
        $resp = Invoke-WebRequest -Uri "https://github.com/$Owner/$Repo/releases/latest" -MaximumRedirection 0 -ErrorAction SilentlyContinue
        if ($resp.Headers.Location -match 'tag/([^/]+)$') { return $matches[1] }
    } catch {}

    throw "Could not determine latest version of $Owner/$Repo"
}

# === Download helpers ===
function Get-FileWithRetry {
    param([string]$Url, [string]$OutPath, [int]$MaxRetries = 3, [int]$TimeoutSec = 120)
    for ($i = 0; $i -lt $MaxRetries; $i++) {
        try {
            Invoke-WebRequest -Uri $Url -OutFile $OutPath -TimeoutSec $TimeoutSec -UseBasicParsing
            return $true
        } catch {
            if ($i -eq $MaxRetries - 1) { return $false }
            Start-Sleep 3
        }
    }
    return $false
}

# === Binary install ===
function Install-BinaryAtomic {
    param([string]$SourcePath, [string]$DestPath)
    $tmp = "$DestPath.tmp.$PID"
    Copy-Item -LiteralPath $SourcePath -Destination $tmp -Force

    $destDir = Split-Path -Parent $DestPath
    $oldFile = Join-Path $destDir "$BinaryName.old.$PID"

    Remove-Item -LiteralPath $DestPath -Force -ErrorAction SilentlyContinue
    Move-Item -LiteralPath $tmp -Destination $DestPath -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $DestPath) { return }

    Rename-Item -LiteralPath $DestPath -NewName $oldFile -Force -ErrorAction SilentlyContinue
    Move-Item -LiteralPath $tmp -Destination $DestPath -Force -ErrorAction SilentlyContinue

    if (-not (Test-Path -LiteralPath $DestPath)) {
        Move-Item -LiteralPath $oldFile -Destination $DestPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue
        throw "Install failed: could not replace $DestPath (binary in use?)"
    }
}

# === PATH persistence ===
function Set-PathPersistence {
    param([string]$Dir, [string]$Scope)

    if ($Scope -eq 'None') { return }

    $dir = $Dir.TrimEnd('\')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -and ($userPath -split ';' -contains $dir)) {
        Write-Info "$dir already on PATH"
        return
    }

    $newUserPath = if ($userPath) { "$dir;$userPath" } else { $dir }

    if ($Scope -eq 'User') {
        [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
        Write-Warn "Added $dir to User PATH. Restart terminal or log out/in for changes to take effect."
    } else {
        $profilePath = $PROFILE
        $profileDir = Split-Path -Parent $profilePath
        if (-not (Test-Path $profileDir)) { New-Item -ItemType Directory -Path $profileDir -Force | Out-Null }
        $line = "`$env:PATH = `"$dir;`$env:PATH`"  # added by ms installer"
        Add-Content -Path $profilePath -Value $line -Encoding UTF8
        Write-Warn "Added $dir to PATH in $profilePath. Restart PowerShell to apply."
    }
}

# === SHA256 verification ===
function Get-ExpectedChecksum {
    param([string]$ArchiveUrl)
    $sumsUrl = $ArchiveUrl -replace '[^/]+\.zip$', 'SHA256SUMS.txt'
    $sumsPath = Join-Path $env:TEMP "ms_sha256sums_$PID.txt"
    if (-not (Get-FileWithRetry -Url $sumsUrl -OutPath $sumsPath -MaxRetries 1 -TimeoutSec 30)) {
        return $null
    }
    $archiveName = Split-Path $ArchiveUrl -Leaf
    $expected = $null
    Get-Content $sumsPath | ForEach-Object {
        if ($_ -match '^([a-fA-F0-9]{64})\s+\S*ms[-.]') {
            $expected = $matches[1]
        }
    }
    Remove-Item -LiteralPath $sumsPath -Force -ErrorAction SilentlyContinue
    return $expected
}

# === MCP registration ===
function Merge-Json {
    param([string]$FilePath, [string]$Key, $Value)
    $data = @{}
    if (Test-Path $FilePath) {
        try { $data = Get-Content -Raw $FilePath | ConvertFrom-Json -AsHashtable } catch {}
    }
    $data[$Key] = $Value
    $dir = Split-Path -Parent $FilePath
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    if ($McpDryRun) {
        Write-Warn "[dry-run] Would write to $FilePath"
        return
    }
    $data | ConvertTo-Json -Depth 10 | Set-Content -Path $FilePath -Encoding UTF8
}

function Register-McpProvider {
    param([string]$ConfigPath, [string]$ProviderName, [string]$JsonKey, [string]$BinaryPath)
    $mcpEntry = @{
        $McpName = @{
            command = $BinaryPath
            args    = @('mcp', 'serve')
            env     = @{}
        }
    }
    try {
        Merge-Json -FilePath $ConfigPath -Key $JsonKey -Value $mcpEntry
        Write-Success "Registered $McpName with $ProviderName"
    } catch {
        Write-Warn "Could not register with $ProviderName (config: $ConfigPath): $_"
    }
}

function Unregister-McpProvider {
    param([string]$ConfigPath, [string]$ProviderName, [string]$JsonKey)
    if (-not (Test-Path $ConfigPath)) { return }
    try {
        $data = Get-Content -Raw $ConfigPath | ConvertFrom-Json -AsHashtable
        if ($data.ContainsKey($JsonKey) -and $data[$JsonKey].ContainsKey($McpName)) {
            $data[$JsonKey].Remove($McpName)
            if ($data[$JsonKey].Count -eq 0) { $data.Remove($JsonKey) }
            if ($McpDryRun) {
                Write-Warn "[dry-run] Would remove $McpName from $ProviderName ($ConfigPath)"
                return
            }
            $data | ConvertTo-Json -Depth 10 | Set-Content -Path $ConfigPath -Encoding UTF8
            Write-Success "Unregistered $McpName from $ProviderName"
        }
    } catch {
        Write-Warn "Could not unregister from $ProviderName: $_"
    }
}

function Register-McpClaude {
    param([string]$BinaryPath)
    Register-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.claude.json') -ProviderName 'Claude Code' -JsonKey 'mcpServers' -BinaryPath $BinaryPath
}

function Register-McpCodex {
    param([string]$BinaryPath)
    $configPath = Join-Path $env:USERPROFILE '.codex\config.toml'
    if (-not (Test-Path (Split-Path $configPath -Parent))) { return }
    try {
        $content = ''
        if (Test-Path $configPath) { $content = Get-Content -Raw $configPath }
        $serverBlock = @"

[mcp_servers.$McpName]
type = "stdio"
command = "$BinaryPath"
args = ["mcp", "serve"]
"@
        if ($McpDryRun) {
            Write-Warn "[dry-run] Would append MCP server to $configPath"
            return
        }
        Add-Content -Path $configPath -Value $serverBlock -Encoding UTF8
        Write-Success "Registered $McpName with Codex CLI"
    } catch {
        Write-Warn "Could not register with Codex CLI: $_"
    }
}

function Register-McpCursor {
    param([string]$BinaryPath)
    Register-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.cursor\mcp.json') -ProviderName 'Cursor' -JsonKey 'mcpServers' -BinaryPath $BinaryPath
}

function Register-McpCline {
    param([string]$BinaryPath)
    $clinePath = Join-Path $env:APPDATA 'Code\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json'
    if (Test-Path (Split-Path $clinePath -Parent)) {
        Register-McpProvider -ConfigPath $clinePath -ProviderName 'Cline' -JsonKey 'mcpServers' -BinaryPath $BinaryPath
    }
}

function Register-McpOpenCode {
    param([string]$BinaryPath)
    $paths = @(
        (Join-Path $env:USERPROFILE '.opencode.json'),
        (Join-Path $env:USERPROFILE '.config\opencode\.opencode.json')
    )
    foreach ($p in $paths) {
        if (Test-Path $p) {
            Register-McpProvider -ConfigPath $p -ProviderName 'OpenCode' -JsonKey 'mcpServers' -BinaryPath $BinaryPath
            return
        }
    }
}

function Register-McpContinue {
    param([string]$BinaryPath)
    $paths = @(
        (Join-Path $env:USERPROFILE '.continue\config.json'),
        (Join-Path $env:USERPROFILE '.continue\config.yaml')
    )
    foreach ($p in $paths) {
        if (Test-Path $p) {
            Register-McpProvider -ConfigPath $p -ProviderName 'Continue' -JsonKey 'mcpServers' -BinaryPath $BinaryPath
            return
        }
    }
}

function Invoke-McpRegistration {
    param([string]$BinaryPath)
    $providers = if ($McpProviders -eq 'all') {
        @('claude', 'codex', 'cursor', 'cline', 'opencode', 'continue')
    } else {
        $McpProviders -split ','
    }
    Write-Info "Registering '$McpName' with MCP providers ($($providers -join ', '))..."
    foreach ($p in $providers) {
        switch ($p.Trim()) {
            'claude'   { Register-McpClaude -BinaryPath $BinaryPath }
            'codex'    { Register-McpCodex -BinaryPath $BinaryPath }
            'cursor'   { Register-McpCursor -BinaryPath $BinaryPath }
            'cline'    { Register-McpCline -BinaryPath $BinaryPath }
            'opencode' { Register-McpOpenCode -BinaryPath $BinaryPath }
            'continue' { Register-McpContinue -BinaryPath $BinaryPath }
            default    { Write-Warn "Unknown MCP provider: $p" }
        }
    }
}

function Invoke-McpUninstall {
    param([string]$BinaryPath)
    Write-Info "Unregistering '$McpName' from all MCP providers..."
    $providers = @('claude', 'codex', 'cursor', 'cline', 'opencode', 'continue')
    foreach ($p in $providers) {
        switch ($p) {
            'claude'   { Unregister-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.claude.json') -ProviderName 'Claude Code' -JsonKey 'mcpServers' }
            'codex'    { Unregister-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.codex\config.toml') -ProviderName 'Codex CLI' -JsonKey 'mcp_servers' }
            'cursor'   { Unregister-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.cursor\mcp.json') -ProviderName 'Cursor' -JsonKey 'mcpServers' }
            'cline'    { Unregister-McpProvider -ConfigPath (Join-Path $env:APPDATA 'Code\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json') -ProviderName 'Cline' -JsonKey 'mcpServers' }
            'opencode' { Unregister-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.opencode.json') -ProviderName 'OpenCode' -JsonKey 'mcpServers' }
            'continue' { Unregister-McpProvider -ConfigPath (Join-Path $env:USERPROFILE '.continue\config.json') -ProviderName 'Continue' -JsonKey 'mcpServers' }
        }
    }
}

# ============================================================================
# Main
# ============================================================================

function Main {
    # ── Uninstall mode ──────────────────────────────────────────────────────
    if ($Uninstall) {
        $target = Join-Path $InstallDir "$BinaryName.exe"
        if (Test-Path $target) {
            Remove-Item -LiteralPath $target -Force
            Write-Success "removed $target"
        }
        Set-PathPersistence -Dir $InstallDir -Scope 'None'
        Invoke-McpUninstall
        return
    }

    # ── Resolve version ─────────────────────────────────────────────────────
    if (-not $Version) {
        Write-Info "Fetching latest version..."
        $Version = Resolve-LatestVersion
        Write-Info "Latest version: $Version"
    } elseif ($Version -notmatch '^v') {
        $Version = "v$Version"
    }

    # ── Platform ────────────────────────────────────────────────────────────
    $target = Get-Target
    Write-Info "Platform: $target | Version: $Version | InstallDir: $InstallDir"

    # ── MCP-only mode ──────────────────────────────────────────────────────
    if ($McpOnly) {
        $binaryPath = Join-Path $InstallDir "$BinaryName.exe"
        if (-not (Test-Path $binaryPath)) {
            $binaryPath = (Get-Command $BinaryName -ErrorAction SilentlyContinue).Source
        }
        if (-not $binaryPath) {
            Write-Warn "$BinaryName not found on PATH or in $InstallDir. Run without -McpOnly first."
            return
        }
        Invoke-McpRegistration -BinaryPath $binaryPath
        return
    }

    # ── Download archive ────────────────────────────────────────────────────
    $versionForUrl = $Version -replace '^v', ''
    $archiveName = "ms-$versionForUrl-$target.zip"
    $archiveUrl  = "https://github.com/$Owner/$Repo/releases/download/$Version/$archiveName"
    $archivePath = Join-Path $env:TEMP "ms_$PID.zip"

    Write-Info "Downloading $archiveName..."
    if (-not (Get-FileWithRetry -Url $archiveUrl -OutPath $archivePath)) {
        throw "Download failed after retries: $archiveUrl"
    }

    # ── SHA256 verification ────────────────────────────────────────────────
    Write-Info "Verifying checksum..."
    $expected = Get-ExpectedChecksum -ArchiveUrl $archiveUrl
    if ($expected) {
        $actual = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLower()
        if ($actual -ne $expected.ToLower()) {
            Remove-Item -LiteralPath $archivePath -Force
            throw "Checksum mismatch. Expected: $expected, Got: $actual"
        }
        Write-Info "Checksum verified"
    } else {
        Write-Warn "No checksum file found — skipping verification"
    }

    # ── Extract ────────────────────────────────────────────────────────
    Write-Info "Extracting..."
    $extractDir = Join-Path $env:TEMP "ms_extract_$PID"
    try {
        if (Test-Path $extractDir) { Remove-Item -Recurse -Force $extractDir }
        New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
        try {
            Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir -Force
        } catch {
            # Fallback: Shell.Application COM (Windows PowerShell)
            $shell = New-Object -ComObject Shell.Application
            $zip = $shell.Namespace((Resolve-Path $archivePath).Path)
            $shell.Namespace((Resolve-Path $extractDir).Path).CopyHere($zip.Items(), 0x10)
        }

        $binaryFile = Get-ChildItem -LiteralPath $extractDir -Recurse -File -Filter "$BinaryName.exe" | Select-Object -First 1
        if (-not $binaryFile) { throw "Could not find $BinaryName.exe in archive" }

        # ── Install ────────────────────────────────────────────────────────
        if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null }
        $dest = Join-Path $InstallDir "$BinaryName.exe"
        Install-BinaryAtomic -SourcePath $binaryFile.FullName -DestPath $dest
        try { Unblock-File -LiteralPath $dest -ErrorAction SilentlyContinue } catch {}

        # PE header sanity check
        $head = [System.IO.File]::ReadAllBytes($dest)[0..1]
        if (-not ($head[0] -eq 0x4D -and $head[1] -eq 0x5A)) {
            throw "Installed $dest does not have a valid PE header. The download was corrupted."
        }

        Set-PathPersistence -Dir $InstallDir -Scope $PathScope

        # Self-test
        $selfTest = & $dest --version 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "Self-test failed: $selfTest"
        }
        Write-Success "Self-test passed: $($selfTest | Select-Object -First 1)"
    } finally {
        Remove-Item -LiteralPath $archivePath -Force -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force $extractDir -ErrorAction SilentlyContinue
    }

    # ── MCP registration ───────────────────────────────────────────────────
    if (-not $NoMcp) {
        $dest = Join-Path $InstallDir "$BinaryName.exe"
        Invoke-McpRegistration -BinaryPath $dest
    }

    # ── Summary ─────────────────────────────────────────────────────────────
    $dest = Join-Path $InstallDir "$BinaryName.exe"
    Write-Host ""
    Write-Success "ms $Version installed to $dest"
    Write-Host ""
    Write-Host "Quick start (open a new PowerShell window for PATH changes):"
    Write-Host "  ms --help"
    Write-Host "  ms doctor"
    Write-Host "  ms search <query>"
    if (-not $NoMcp) {
        Write-Host "  ms mcp serve        # MCP server (registered with detected agents)"
    }
}

Main
