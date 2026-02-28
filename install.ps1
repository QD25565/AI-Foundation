# AI-Foundation — One-line installer for Windows
#
# Usage:
#   irm https://github.com/QD25565/ai-foundation/raw/main/install.ps1 | iex
#
# Or save and run with options:
#   .\install.ps1                          # Install latest version
#   .\install.ps1 -Version 57             # Install specific version
#   .\install.ps1 -BinDir "C:\custom"     # Custom install directory
#   .\install.ps1 -SkipPath               # Skip PATH modification

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$BinDir = "",
    [switch]$SkipPath
)

$ErrorActionPreference = "Stop"

# ── Configuration ─────────────────────────────────────────
$Repo = "QD25565/ai-foundation"
$GitHub = "https://github.com/$Repo"
$Api = "https://api.github.com/repos/$Repo"

if ($BinDir) {
    $InstallDir = $BinDir
} else {
    $InstallDir = Join-Path $env:USERPROFILE ".ai-foundation\bin"
}

# ── Formatting ────────────────────────────────────────────
function Write-OK($msg)   { Write-Host "  $([char]0x2713) $msg" -ForegroundColor Green }
function Write-Info($msg)  { Write-Host "  -> $msg" -ForegroundColor Cyan }
function Write-Warn($msg)  { Write-Host "  ! $msg" -ForegroundColor Yellow }
function Write-Err($msg)   { Write-Host "  X $msg" -ForegroundColor Red }
function Write-Die($msg)   { Write-Err $msg; exit 1 }

function Show-Banner {
    Write-Host ""
    Write-Host "     AAAAA  II" -ForegroundColor White
    Write-Host "    AA   AA II" -ForegroundColor White
    Write-Host "    AAAAAAA II" -ForegroundColor White
    Write-Host "    AA   AA II" -ForegroundColor White
    Write-Host "    AA   AA II" -ForegroundColor White
    Write-Host ""
    Write-Host "    F O U N D A T I O N" -ForegroundColor White
    Write-Host ""
}

# ── Version Resolution ────────────────────────────────────
function Get-LatestVersion {
    if ($Version) {
        Write-Info "Using specified version: v$Version"
        return $Version
    }

    Write-Info "Fetching latest version..."

    try {
        $headers = @{ "User-Agent" = "AI-Foundation-Installer/1.0" }
        $response = Invoke-RestMethod -Uri "$Api/releases/latest" -Headers $headers -TimeoutSec 15
        $tag = $response.tag_name -replace '^v', ''
        if ($tag) {
            Write-OK "Latest version: v$tag"
            return $tag
        }
    } catch {
        # API failed, try version.txt
    }

    try {
        $versionText = Invoke-RestMethod -Uri "$GitHub/raw/main/version.txt" -TimeoutSec 10
        $ver = $versionText.Trim()
        if ($ver) {
            Write-OK "Version from repo: v$ver"
            return $ver
        }
    } catch {
        # Both failed
    }

    Write-Die "Could not determine latest version. Use -Version to specify manually."
}

# ── Download & Extract ────────────────────────────────────
function Install-Binaries($ResolvedVersion) {
    $archiveName = "ai-foundation-v$ResolvedVersion-windows-x64.zip"
    $url = "$GitHub/releases/download/v$ResolvedVersion/$archiveName"
    $tempDir = Join-Path $env:TEMP "ai-foundation-install-$(Get-Random)"
    $archivePath = Join-Path $tempDir $archiveName

    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    Write-Info "Downloading: $archiveName"
    try {
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri $url -OutFile $archivePath -UseBasicParsing -TimeoutSec 120
    } catch {
        Write-Die "Download failed. Check that v$ResolvedVersion has a Windows release at: $url"
    }

    # Verify file size
    $size = (Get-Item $archivePath).Length
    if ($size -lt 10000) {
        Write-Die "Downloaded file too small ($size bytes) - likely a 404."
    }

    $sizeMB = [math]::Round($size / 1MB, 1)
    Write-OK "Downloaded: $sizeMB MB"

    # SHA256
    $hash = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash
    Write-Info "SHA256: $hash"

    # Extract
    Write-Info "Extracting to $InstallDir"
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null

    $extractDir = Join-Path $tempDir "extracted"
    Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

    # Find binaries (inside ai-foundation-vXX\bin\ or ai-foundation-vXX\ prefix)
    $count = 0
    $binSearchPaths = @(
        (Join-Path $extractDir "ai-foundation-v$ResolvedVersion" "bin"),
        (Join-Path $extractDir "ai-foundation-v$ResolvedVersion"),
        $extractDir
    )

    foreach ($searchPath in $binSearchPaths) {
        if (Test-Path $searchPath) {
            $files = Get-ChildItem -Path $searchPath -File -Filter "*.exe" -ErrorAction SilentlyContinue
            foreach ($file in $files) {
                Copy-Item -Path $file.FullName -Destination $InstallDir -Force
                $count++
            }
            if ($count -gt 0) { break }
        }
    }

    if ($count -eq 0) {
        Write-Die "No .exe files found in archive"
    }

    Write-OK "Installed $count binaries to $InstallDir"

    # Write VERSION file
    Set-Content -Path (Join-Path $InstallDir "VERSION") -Value $ResolvedVersion -NoNewline

    # Cleanup
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ── PATH Setup ────────────────────────────────────────────
function Set-UserPath {
    if ($SkipPath) {
        Write-Info "Skipping PATH setup (-SkipPath)"
        return
    }

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -split ";" | Where-Object { $_ -eq $InstallDir }) {
        Write-OK "Already in PATH"
        return
    }

    $newPath = "$InstallDir;$currentPath"
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")

    # Also update current session
    $env:Path = "$InstallDir;$env:Path"

    Write-OK "Added to User PATH: $InstallDir"
    Write-Info "PATH is updated for new terminals. Current session also updated."
}

# ── Main ──────────────────────────────────────────────────
function Main {
    Show-Banner

    Write-Info "Platform: Windows x64"
    $resolvedVersion = Get-LatestVersion
    Install-Binaries $resolvedVersion

    Set-UserPath

    # Try starting the daemon
    $daemonPath = Join-Path $InstallDir "v2-daemon.exe"
    if (Test-Path $daemonPath) {
        Write-Info "Starting daemon..."
        try {
            Start-Process -FilePath $daemonPath -WindowStyle Hidden
            Write-OK "Daemon started"
        } catch {
            Write-Warn "Could not start daemon: $_"
        }
    }

    # Summary
    Write-Host ""
    Write-Host "  Installation Complete!" -ForegroundColor Green
    Write-Host ""
    Write-Host "  Binaries: " -NoNewline -ForegroundColor Cyan; Write-Host $InstallDir
    Write-Host "  Version:  " -NoNewline -ForegroundColor Cyan; Write-Host "v$resolvedVersion"
    Write-Host ""
    Write-Host "  Next steps:" -ForegroundColor White
    Write-Host "    1. Open a new terminal (PATH is already updated)"
    Write-Host "    2. Run: " -NoNewline; Write-Host "ai-foundation-mcp --help" -ForegroundColor Cyan
    Write-Host "    3. For full setup (project config, AI_ID, forge):"
    Write-Host "       " -NoNewline; Write-Host "git clone $GitHub && cd ai-foundation && python install.py" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "  Docs: $GitHub" -ForegroundColor Cyan
    Write-Host ""
}

Main
