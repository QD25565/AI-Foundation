@echo off
REM Health Monitor Startup Script
REM Launches the Teambook Health Monitor with real-time data visualization

echo ============================================================
echo TEAMBOOK HEALTH MONITOR
echo ============================================================
echo.
echo Starting server...
echo.

cd /d "%~dp0"

REM Start Python server
start /B python health_server.py

REM Wait a moment for server to start
timeout /t 3 /nobreak >nul

REM Open browser
start http://localhost:8765

echo.
echo Health Monitor launched!
echo.
echo Server running at: http://localhost:8765
echo Press Ctrl+C in the server window to stop
echo.
echo ============================================================
