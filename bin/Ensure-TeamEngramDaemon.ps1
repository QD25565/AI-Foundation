# AI-Foundation TeamEngram Daemon - Ensure Running (PowerShell)
# More robust version with proper process detection and startup verification

param(
    [switch]$Verbose,
    [int]$TimeoutSeconds = 5
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$DaemonPath = Join-Path $ScriptDir "teamengram-daemon.exe"
$PipeName = "teamengram"

function Test-DaemonRunning {
    $process = Get-Process -Name "teamengram-daemon" -ErrorAction SilentlyContinue
    return $null -ne $process
}

function Test-PipeExists {
    return Test-Path "\\.\pipe\$PipeName"
}

function Start-Daemon {
    if ($Verbose) { Write-Host "Starting TeamEngram daemon..." }
    Start-Process -FilePath $DaemonPath -WindowStyle Hidden
}

# Check if daemon executable exists
if (-not (Test-Path $DaemonPath)) {
    Write-Error "Daemon not found at: $DaemonPath"
    exit 1
}

# Check if already running
if (Test-DaemonRunning) {
    if ($Verbose) { Write-Host "Daemon already running" }
    exit 0
}

# Start daemon
Start-Daemon

# Wait for startup with timeout
$elapsed = 0
$interval = 0.25
while ($elapsed -lt $TimeoutSeconds) {
    Start-Sleep -Milliseconds 250
    $elapsed += $interval

    if (Test-DaemonRunning -and Test-PipeExists) {
        if ($Verbose) { Write-Host "Daemon started successfully (${elapsed}s)" }
        exit 0
    }
}

# Timeout
Write-Error "Daemon failed to start within ${TimeoutSeconds}s"
exit 1
