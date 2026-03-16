@echo off
setlocal enabledelayedexpansion

echo.
echo ============================================================
echo    AI FOUNDATION - COVE CONNECTOR STARTUP
echo ============================================================
echo.

:: Set paths
set BIN_DIR=%~dp0
set MCP_SERVER=%BIN_DIR%ai-foundation-mcp-http.exe
:: Try ngrok from PATH first, fall back to NGROK_PATH env var
where ngrok >nul 2>&1
if not errorlevel 1 (
    for /f "delims=" %%i in ('where ngrok') do set NGROK=%%i
) else if defined NGROK_PATH (
    set NGROK=%NGROK_PATH%
) else (
    echo [ERROR] ngrok not found in PATH and NGROK_PATH env var is not set.
    echo Install ngrok and add it to PATH, or set NGROK_PATH to the ngrok executable.
    pause
    exit /b 1
)
set PORT=8080

:: Check if ngrok is configured
%NGROK% config check >nul 2>&1
if errorlevel 1 (
    echo [WARNING] ngrok is not configured with an auth token.
    echo.
    echo To get a FREE auth token:
    echo   1. Go to https://dashboard.ngrok.com/signup
    echo   2. Sign up (free, no credit card)
    echo   3. Copy your authtoken from https://dashboard.ngrok.com/get-started/your-authtoken
    echo   4. Run: ngrok config add-authtoken YOUR_TOKEN
    echo.
    echo Press any key to continue anyway (may have limitations)...
    pause >nul
)

:: Check if MCP server exists
if not exist "%MCP_SERVER%" (
    echo [ERROR] MCP server not found at: %MCP_SERVER%
    echo Please build it first.
    pause
    exit /b 1
)

:: Set environment variables for the MCP server
set AI_ID=cove-web
if not defined POSTGRES_URL set POSTGRES_URL=postgresql://ai_foundation:changeme@127.0.0.1:5432/ai_foundation

echo [1/3] Starting AI Foundation MCP Server on port %PORT%...
start "AI-Foundation MCP Server" cmd /c "%MCP_SERVER% --port %PORT%"

:: Wait for server to start
timeout /t 2 /nobreak >nul

:: Check if server is running
powershell -Command "(New-Object System.Net.Sockets.TcpClient).Connect('127.0.0.1', %PORT%)" 2>nul
if errorlevel 1 (
    echo [WAITING] Server starting up...
    timeout /t 3 /nobreak >nul
)

echo [2/3] Starting ngrok tunnel...
start "ngrok" cmd /k "%NGROK% http %PORT% --log=stdout"

:: Wait for ngrok to establish tunnel
echo [3/3] Waiting for ngrok tunnel...
timeout /t 5 /nobreak >nul

:: Get the ngrok URL
echo.
echo ============================================================
echo    SETUP COMPLETE!
echo ============================================================
echo.
echo The ngrok window shows your public URL.
echo Look for a line like: Forwarding https://xxxx.ngrok-free.app
echo.
echo To connect Cove (Claude Web):
echo   1. Go to Claude.ai Settings ^> Connectors
echo   2. Click "Add custom connector"
echo   3. Name: AI Foundation
echo   4. URL: https://YOUR-NGROK-URL/mcp
echo   5. Leave OAuth fields empty
echo   6. Click Add
echo.
echo Available tools for Cove:
echo   - notebook_remember: Save notes to persistent memory
echo   - notebook_recall: Search notes
echo   - notebook_list: List recent notes
echo   - notebook_stats: Get memory statistics
echo   - identity_whoami: Check connection status
echo.
echo Keep both windows open while using Cove!
echo Press any key to close this setup window...
pause >nul
