@echo off
REM TeamEngram Daemon Auto-Start Setup
REM Creates a Windows Task Scheduler task to start daemon on user login (HIDDEN)
REM Run this script once as Administrator

setlocal

REM Get the directory where this script is located
set SCRIPT_DIR=%~dp0
set DAEMON_PATH=%SCRIPT_DIR%..\bin\teamengram-daemon.exe
set VBS_PATH=%SCRIPT_DIR%..\bin\start-daemon-hidden.vbs

REM Normalize the paths
for %%I in ("%DAEMON_PATH%") do set DAEMON_PATH=%%~fI
for %%I in ("%VBS_PATH%") do set VBS_PATH=%%~fI

echo TeamEngram Daemon Auto-Start Setup (HIDDEN MODE)
echo =================================================
echo.
echo Daemon path: %DAEMON_PATH%
echo.

REM Check if daemon exists
if not exist "%DAEMON_PATH%" (
    echo ERROR: Daemon not found at %DAEMON_PATH%
    echo Please ensure teamengram-daemon.exe is in the bin folder.
    pause
    exit /b 1
)

REM Create VBScript launcher for hidden execution
echo Creating hidden launcher...
(
echo Set WshShell = CreateObject^("WScript.Shell"^)
echo WshShell.Run """%DAEMON_PATH%""", 0, False
) > "%VBS_PATH%"

REM Delete existing task if it exists (ignore errors)
schtasks /delete /tn "TeamEngramDaemon" /f >nul 2>&1

REM Create the scheduled task using VBScript for hidden window
echo Creating scheduled task (hidden window)...
schtasks /create /tn "TeamEngramDaemon" /tr "wscript.exe \"%VBS_PATH%\"" /sc onlogon /rl highest /f

if %errorlevel% neq 0 (
    echo.
    echo ERROR: Failed to create scheduled task.
    echo Please run this script as Administrator.
    pause
    exit /b 1
)

echo.
echo SUCCESS: Task scheduler entry created (HIDDEN MODE).
echo The TeamEngram daemon will start automatically on login with NO visible window.
echo.
echo To verify: Open Task Scheduler and look for "TeamEngramDaemon"
echo To remove: schtasks /delete /tn "TeamEngramDaemon" /f
echo.

REM Start the daemon now if not already running (hidden)
tasklist /fi "imagename eq teamengram-daemon.exe" 2>nul | find /i "teamengram-daemon.exe" >nul
if %errorlevel% neq 0 (
    echo Starting daemon now (hidden)...
    wscript.exe "%VBS_PATH%"
    timeout /t 2 >nul
    tasklist /fi "imagename eq teamengram-daemon.exe" 2>nul | find /i "teamengram-daemon.exe" >nul
    if %errorlevel% equ 0 (
        echo Daemon started successfully (hidden).
    ) else (
        echo WARNING: Daemon may not have started. Check Task Manager.
    )
) else (
    echo Daemon is already running.
)

echo.
echo Setup complete! You can close any existing daemon terminal windows.
echo The daemon will continue running in the background.
pause
