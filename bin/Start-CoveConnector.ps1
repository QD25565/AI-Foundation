# AI Foundation - Cove Connector Startup Script
# Starts MCP HTTP server + Cloudflare Tunnel
# Permanent URL: https://mcp.fitquestapp.org/mcp

$ErrorActionPreference = "Continue"

# Config
$BinDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$McpServer = Join-Path $BinDir "ai-foundation-mcp-http.exe"
$Cloudflared = "C:\Program Files (x86)\cloudflared\cloudflared.exe"
$Port = 8080
$McpUrl = "https://mcp.fitquestapp.org/mcp"

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "   AI FOUNDATION - COVE CONNECTOR" -ForegroundColor Cyan
Write-Host "   Powered by Cloudflare Tunnel" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host ""

# Check binaries exist
if (-not (Test-Path $McpServer)) {
    Write-Host "[ERROR] MCP server not found: $McpServer" -ForegroundColor Red
    exit 1
}
if (-not (Test-Path $Cloudflared)) {
    Write-Host "[ERROR] cloudflared not found." -ForegroundColor Red
    exit 1
}

# Set environment for MCP server
$env:AI_ID = "cove-web"
$env:POSTGRES_URL = "postgresql://ai_foundation:ai_foundation_pass@127.0.0.1:5432/ai_foundation"

# Kill any existing processes
Write-Host "[1/3] Cleaning up existing processes..." -ForegroundColor Yellow
Get-Process -Name "ai-foundation-mcp-http" -ErrorAction SilentlyContinue | Stop-Process -Force
Get-Process -Name "cloudflared" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1

# Start MCP server
Write-Host "[2/3] Starting AI Foundation MCP Server on port $Port..." -ForegroundColor Yellow
$mcpProcess = Start-Process -FilePath $McpServer -ArgumentList "--port", $Port -PassThru -WindowStyle Minimized
Start-Sleep -Seconds 2

# Verify server is up
$maxRetries = 10
$serverUp = $false
for ($i = 0; $i -lt $maxRetries; $i++) {
    try {
        $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/health" -TimeoutSec 2 -ErrorAction Stop
        $serverUp = $true
        break
    } catch {
        Write-Host "   Waiting for server... ($($i+1)/$maxRetries)" -ForegroundColor Gray
        Start-Sleep -Seconds 1
    }
}

if (-not $serverUp) {
    Write-Host "[ERROR] MCP server failed to start!" -ForegroundColor Red
    exit 1
}
Write-Host "   Server is running!" -ForegroundColor Green

# Start Cloudflare Tunnel
Write-Host "[3/3] Starting Cloudflare Tunnel..." -ForegroundColor Yellow
$cloudflaredProcess = Start-Process -FilePath $Cloudflared -ArgumentList "tunnel", "run" -PassThru -WindowStyle Minimized
Start-Sleep -Seconds 3

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host "   COVE CONNECTOR READY!" -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host ""
Write-Host "Permanent MCP URL:" -ForegroundColor White
Write-Host ""
Write-Host "   $McpUrl" -ForegroundColor Cyan
Write-Host ""

# Copy to clipboard
Set-Clipboard -Value $McpUrl
Write-Host "(Copied to clipboard!)" -ForegroundColor Green
Write-Host ""

Write-Host "To connect Cove (Claude Web) - ONE TIME SETUP:" -ForegroundColor White
Write-Host "  1. Go to Claude.ai -> Settings -> Connectors" -ForegroundColor Gray
Write-Host "  2. Click 'Add custom connector'" -ForegroundColor Gray
Write-Host "  3. Name: AI Foundation" -ForegroundColor Gray
Write-Host "  4. URL: $McpUrl" -ForegroundColor Cyan
Write-Host "  5. Leave OAuth fields empty" -ForegroundColor Gray
Write-Host "  6. Click Add" -ForegroundColor Gray
Write-Host ""
Write-Host "Once configured, Cove will always connect to this URL!" -ForegroundColor Green
Write-Host ""
Write-Host "Tools available to Cove:" -ForegroundColor White
Write-Host "  notebook_remember - Save notes to persistent memory" -ForegroundColor Gray
Write-Host "  notebook_recall   - Search notes (hybrid vector+keyword)" -ForegroundColor Gray
Write-Host "  notebook_list     - List recent notes" -ForegroundColor Gray
Write-Host "  notebook_stats    - Memory statistics" -ForegroundColor Gray
Write-Host "  identity_whoami   - Connection status" -ForegroundColor Gray
Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "Press Ctrl+C to stop" -ForegroundColor Yellow
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host ""

# Keep running and monitor
try {
    while ($true) {
        if ($mcpProcess.HasExited) {
            Write-Host "[ERROR] MCP server stopped!" -ForegroundColor Red
            break
        }
        if ($cloudflaredProcess.HasExited) {
            Write-Host "[ERROR] Cloudflare tunnel stopped!" -ForegroundColor Red
            break
        }
        Start-Sleep -Seconds 5
    }
} finally {
    Write-Host ""
    Write-Host "Shutting down..." -ForegroundColor Yellow
    Stop-Process -Id $mcpProcess.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $cloudflaredProcess.Id -Force -ErrorAction SilentlyContinue
    Write-Host "Done." -ForegroundColor Green
}
