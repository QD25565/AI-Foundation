<#
.SYNOPSIS
    AI-Foundation Tool Deployment Script
    Deploys binaries from All Tools/bin to all AI instance locations.

.DESCRIPTION
    Prevents tool drift by ensuring all AI instances have the same binaries.
    Supports deploying specific files or all tools at once.

.PARAMETER Files
    Specific file(s) to deploy. If omitted, deploys all .exe files.

.PARAMETER DryRun
    Shows what would be copied without actually copying.

.PARAMETER Force
    Kill running processes before copying (use with caution).

.EXAMPLE
    .\deploy-tools.ps1
    Deploys all binaries to all instances.

.EXAMPLE
    .\deploy-tools.ps1 -Files teamengram-daemon.exe,teambook.exe
    Deploys specific binaries.

.EXAMPLE
    .\deploy-tools.ps1 -DryRun
    Shows what would be deployed without copying.

.EXAMPLE
    .\deploy-tools.ps1 -Files teamengram-daemon.exe -Force
    Kills running daemon processes and deploys.
#>

param(
    [string[]]$Files,
    [switch]$DryRun,
    [switch]$Force
)

# ============================================
# CONFIGURATION - All AI Instance Locations
# ============================================

# Source directory: the bin/ folder next to this script
$SourceDir = Join-Path $PSScriptRoot "bin"

# Base directory for AI instances (parent of instance folders)
# Override by setting $env:AI_INSTANCES_DIR before running, or edit this default.
$InstancesDir = if ($env:AI_INSTANCES_DIR) { $env:AI_INSTANCES_DIR } else { Split-Path $PSScriptRoot -Parent }

# Additional target directories (e.g. project-specific agents)
# Override by setting $env:AI_EXTRA_TARGETS as semicolon-separated "Name=Path" pairs.
# Example: $env:AI_EXTRA_TARGETS = "fitquest-aurora=C:\Projects\FitQuest2\aurora\bin;fitquest-crystal=C:\Projects\FitQuest2\crystal\bin"
$ExtraTargets = @()
if ($env:AI_EXTRA_TARGETS) {
    foreach ($entry in $env:AI_EXTRA_TARGETS -split ";") {
        $parts = $entry -split "=", 2
        if ($parts.Count -eq 2) {
            $ExtraTargets += @{ Name = $parts[0].Trim(); Path = $parts[1].Trim() }
        }
    }
}

$Targets = @(
    # Claude Code Instances (auto-discovered from $InstancesDir)
    @{ Name = "claude-code-instance-1"; Path = Join-Path $InstancesDir "claude-code-instance-1\bin" },
    @{ Name = "claude-code-instance-2"; Path = Join-Path $InstancesDir "claude-code-instance-2\bin" },
    @{ Name = "claude-code-instance-3"; Path = Join-Path $InstancesDir "claude-code-instance-3\bin" },
    @{ Name = "claude-code-instance-4"; Path = Join-Path $InstancesDir "claude-code-instance-4\bin" },

    # Gemini CLI Instance
    @{ Name = "gemini-cli-instance-1"; Path = Join-Path $InstancesDir "gemini-cli-instance-1\bin" }
) + $ExtraTargets

# Core tools that should always be deployed (subset for quick updates)
$CoreTools = @(
    "notebook-cli.exe",
    "teambook.exe",
    "teamengram-daemon.exe",
    "session-start.exe",
    "hook-bulletin.exe",
    "ai-foundation-mcp.exe"
)

# ============================================
# FUNCTIONS
# ============================================

function Write-Status {
    param([string]$Message, [string]$Status = "INFO")
    $color = switch ($Status) {
        "OK" { "Green" }
        "FAIL" { "Red" }
        "SKIP" { "Yellow" }
        "INFO" { "Cyan" }
        default { "White" }
    }
    Write-Host "[$Status] " -ForegroundColor $color -NoNewline
    Write-Host $Message
}

function Test-ProcessRunning {
    param([string]$ProcessName)
    $name = [System.IO.Path]::GetFileNameWithoutExtension($ProcessName)
    return (Get-Process -Name $name -ErrorAction SilentlyContinue) -ne $null
}

function Stop-ToolProcess {
    param([string]$ProcessName)
    $name = [System.IO.Path]::GetFileNameWithoutExtension($ProcessName)
    $procs = Get-Process -Name $name -ErrorAction SilentlyContinue
    if ($procs) {
        Write-Status "Stopping $name processes..." "INFO"
        $procs | Stop-Process -Force
        Start-Sleep -Milliseconds 500
        return $true
    }
    return $false
}

function Deploy-File {
    param(
        [string]$SourceFile,
        [string]$TargetDir,
        [string]$TargetName,
        [switch]$DryRun,
        [switch]$Force
    )

    $fileName = [System.IO.Path]::GetFileName($SourceFile)
    $targetPath = Join-Path $TargetDir $fileName

    # Check if target directory exists
    if (-not (Test-Path $TargetDir)) {
        if ($DryRun) {
            Write-Status "Would create directory: $TargetDir" "INFO"
        } else {
            New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null
            Write-Status "Created directory: $TargetDir" "OK"
        }
    }

    # Check if process is running
    if (Test-ProcessRunning $fileName) {
        if ($Force) {
            if (-not $DryRun) {
                Stop-ToolProcess $fileName
            } else {
                Write-Status "Would stop process: $fileName" "INFO"
            }
        } else {
            Write-Status "$TargetName/$fileName - Process running (use -Force)" "SKIP"
            return $false
        }
    }

    # Compare files
    if (Test-Path $targetPath) {
        $sourceHash = (Get-FileHash $SourceFile -Algorithm MD5).Hash
        $targetHash = (Get-FileHash $targetPath -Algorithm MD5).Hash
        if ($sourceHash -eq $targetHash) {
            Write-Status "$TargetName/$fileName - Already up to date" "SKIP"
            return $true
        }
    }

    # Copy file
    if ($DryRun) {
        Write-Status "Would copy: $fileName -> $TargetName" "INFO"
    } else {
        try {
            Copy-Item -Path $SourceFile -Destination $targetPath -Force
            Write-Status "$TargetName/$fileName - Deployed" "OK"
        } catch {
            Write-Status "$TargetName/$fileName - FAILED: $($_.Exception.Message)" "FAIL"
            return $false
        }
    }
    return $true
}

# ============================================
# MAIN EXECUTION
# ============================================

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  AI-Foundation Tool Deployment" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

if ($DryRun) {
    Write-Host "[DRY RUN MODE - No changes will be made]" -ForegroundColor Yellow
    Write-Host ""
}

# Determine which files to deploy
if ($Files) {
    $filesToDeploy = $Files
    Write-Status "Deploying specified files: $($Files -join ', ')" "INFO"
} else {
    $filesToDeploy = Get-ChildItem -Path $SourceDir -Filter "*.exe" | Select-Object -ExpandProperty Name
    Write-Status "Deploying all $($filesToDeploy.Count) executables from source" "INFO"
}

Write-Host ""

# Track results
$results = @{
    Success = 0
    Skipped = 0
    Failed = 0
}

# Deploy to each target
foreach ($target in $Targets) {
    Write-Host "--- $($target.Name) ---" -ForegroundColor Magenta

    foreach ($file in $filesToDeploy) {
        $sourcePath = Join-Path $SourceDir $file

        if (-not (Test-Path $sourcePath)) {
            Write-Status "$file - Source not found" "FAIL"
            $results.Failed++
            continue
        }

        $result = Deploy-File -SourceFile $sourcePath -TargetDir $target.Path -TargetName $target.Name -DryRun:$DryRun -Force:$Force

        if ($result -eq $true) {
            $results.Success++
        } elseif ($result -eq $false) {
            $results.Failed++
        }
    }
    Write-Host ""
}

# Summary
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Deployment Summary" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Targets: $($Targets.Count)" -ForegroundColor White
Write-Host "  Files per target: $($filesToDeploy.Count)" -ForegroundColor White
Write-Host "  Deployed: $($results.Success)" -ForegroundColor Green
Write-Host "  Skipped (up to date): $($results.Skipped)" -ForegroundColor Yellow
Write-Host "  Failed: $($results.Failed)" -ForegroundColor $(if ($results.Failed -gt 0) { "Red" } else { "Green" })
Write-Host ""

if ($results.Failed -gt 0) {
    exit 1
}
