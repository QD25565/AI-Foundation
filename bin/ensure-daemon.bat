@echo off
:: AI-Foundation TeamEngram Daemon - Ensure Running
:: This script ensures the daemon is running before AI clients connect.
:: Can be called by any launcher or manually.
:: Silent operation - exits 0 if daemon running, 1 if failed to start.

setlocal

:: Get script directory
set "SCRIPT_DIR=%~dp0"
set "DAEMON_PATH=%SCRIPT_DIR%teamengram-daemon.exe"

:: Check if daemon exists
if not exist "%DAEMON_PATH%" (
    echo ERROR: Daemon not found at %DAEMON_PATH% 1>&2
    exit /b 1
)

:: Check if daemon is already running
tasklist /fi "imagename eq teamengram-daemon.exe" 2>nul | find /i "teamengram-daemon.exe" >nul
if %errorlevel% equ 0 (
    :: Daemon already running
    exit /b 0
)

:: Daemon not running - start it
start "" /b "%DAEMON_PATH%"

:: Wait a moment for startup
timeout /t 1 /nobreak >nul

:: Verify it started
tasklist /fi "imagename eq teamengram-daemon.exe" 2>nul | find /i "teamengram-daemon.exe" >nul
if %errorlevel% equ 0 (
    exit /b 0
) else (
    echo ERROR: Failed to start daemon 1>&2
    exit /b 1
)
