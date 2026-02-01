@echo off
:: AI-Foundation TeamEngram Daemon Auto-Start Setup
:: This script registers the daemon to start automatically at user login

setlocal enabledelayedexpansion

echo.
echo ========================================
echo  TeamEngram Daemon Auto-Start Setup
echo ========================================
echo.

:: Get the directory where this script lives
set "SCRIPT_DIR=%~dp0"
set "DAEMON_PATH=%SCRIPT_DIR%teamengram-daemon.exe"

:: Verify daemon exists
if not exist "%DAEMON_PATH%" (
    echo ERROR: teamengram-daemon.exe not found at:
    echo   %DAEMON_PATH%
    echo.
    pause
    exit /b 1
)

echo Found daemon at: %DAEMON_PATH%
echo.

:: Check if task already exists
schtasks /query /tn "TeamEngramDaemon" >nul 2>&1
if %errorlevel% equ 0 (
    echo Task "TeamEngramDaemon" already exists.
    set /p "REPLACE=Replace existing task? (y/n): "
    if /i "!REPLACE!" neq "y" (
        echo Keeping existing task.
        goto :check_running
    )
    schtasks /delete /tn "TeamEngramDaemon" /f >nul 2>&1
)

:: Create scheduled task to run at user logon
echo Creating scheduled task...
schtasks /create /tn "TeamEngramDaemon" /tr "\"%DAEMON_PATH%\"" /sc onlogon /rl highest /f

if %errorlevel% equ 0 (
    echo.
    echo SUCCESS: Daemon will auto-start at login
) else (
    echo.
    echo WARNING: Could not create scheduled task.
    echo You may need to run this as Administrator.
)

:check_running
echo.
echo ----------------------------------------

:: Check if daemon is currently running
tasklist /fi "imagename eq teamengram-daemon.exe" 2>nul | find /i "teamengram-daemon.exe" >nul
if %errorlevel% equ 0 (
    echo Daemon is currently RUNNING
) else (
    echo Daemon is NOT running
    set /p "START_NOW=Start daemon now? (y/n): "
    if /i "!START_NOW!" equ "y" (
        echo Starting daemon...
        start "" "%DAEMON_PATH%"
        timeout /t 2 /nobreak >nul
        echo Daemon started.
    )
)

echo.
echo ========================================
echo  Setup Complete
echo ========================================
echo.
echo The daemon will now:
echo   1. Start automatically when you log in
echo   2. Stay running in the background
echo   3. Serve all AI clients (Claude, Gemini, etc.)
echo.
pause
