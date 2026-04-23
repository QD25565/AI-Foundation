"""v2-daemon management — start, check, and set up auto-start."""

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

from .platform import Platform, binary_ext
from .ui import ok, info, warn, step


def is_running(platform: Platform) -> bool:
    """Check if v2-daemon is currently running."""
    ext = binary_ext(platform)
    binary_name = f"v2-daemon{ext}"
    try:
        if platform == Platform.WINDOWS:
            result = subprocess.run(
                ["tasklist", "/fi", f"imagename eq {binary_name}"],
                capture_output=True, text=True, timeout=5
            )
            return binary_name.lower() in result.stdout.lower()
        else:
            result = subprocess.run(
                ["pgrep", "-f", "v2-daemon"],
                capture_output=True, text=True, timeout=5
            )
            return result.returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return False


def start(bin_dir: Path, platform: Platform) -> bool:
    """Start v2-daemon in the background. Returns True if started or already running."""
    if is_running(platform):
        ok("Daemon already running")
        return True

    ext = binary_ext(platform)
    daemon_path = bin_dir / f"v2-daemon{ext}"
    if not daemon_path.exists():
        warn(f"v2-daemon not found at {daemon_path}")
        return False

    step("Starting v2-daemon")
    try:
        if platform == Platform.WINDOWS:
            # Hidden window, detached from current console
            si = subprocess.STARTUPINFO()
            si.dwFlags |= subprocess.STARTF_USESHOWWINDOW
            si.wShowWindow = 0  # SW_HIDE
            subprocess.Popen(
                [str(daemon_path)],
                startupinfo=si,
                creationflags=subprocess.DETACHED_PROCESS,
                close_fds=True
            )
        else:
            subprocess.Popen(
                [str(daemon_path)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True
            )

        # Wait up to 3s for daemon to be ready
        for _ in range(6):
            time.sleep(0.5)
            if is_running(platform):
                ok("Daemon started")
                return True

        warn("Daemon may not have started — check manually with: v2-daemon --help")
        return False

    except OSError as e:
        warn(f"Could not start daemon: {e}")
        return False


def setup_autostart(bin_dir: Path, platform: Platform, yes: bool = False) -> None:
    """Set up v2-daemon to start automatically on login."""
    step("Daemon auto-start")

    if platform == Platform.WINDOWS:
        _setup_autostart_windows(bin_dir)
    elif platform == Platform.WSL:
        # Auto-start in WSL runs the Windows binary via the Windows Startup folder
        _setup_autostart_windows(bin_dir)
    elif platform == Platform.LINUX:
        _setup_autostart_systemd(bin_dir)
    elif platform == Platform.MACOS:
        _setup_autostart_launchd(bin_dir)


_WIN_TASK_NAME = "AI-Foundation-Daemon"


def _windows_daemon_exe_path(bin_dir: Path) -> str:
    """Resolve bin_dir / v2-daemon.exe to a native Windows path string.

    bin_dir may be a WSL-style /mnt/c/... path when the installer runs under WSL;
    the scheduled task must embed a native C:\\... path for the Windows task engine.
    """
    daemon_exe = bin_dir / "v2-daemon.exe"
    try:
        result = subprocess.run(
            ["wslpath", "-w", str(daemon_exe)],
            capture_output=True, text=True, timeout=5,
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass
    return str(daemon_exe)


def _run_powershell(script: str, timeout: int = 20) -> subprocess.CompletedProcess:
    """Invoke a PowerShell script block via powershell.exe (works under WSL + native)."""
    return subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive",
         "-ExecutionPolicy", "Bypass", "-Command", script],
        capture_output=True, text=True, timeout=timeout,
    )


def _remove_legacy_startup_vbs() -> None:
    """Delete any earlier-install VBS auto-start artifacts from the Startup folder.

    Older installs of this project (and an interim hand-rolled watchdog) dropped
    VBS launchers into %APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup.
    The Scheduled Task path supersedes all of them; removing the files prevents
    double-launch at login and keeps one source of truth for daemon lifecycle.
    """
    script = r"""
$startup = [Environment]::GetFolderPath('Startup')
$names = @('ai-foundation-daemon.vbs','start-v2-daemon-hidden.vbs','v2-daemon-watchdog.vbs')
foreach ($n in $names) {
    $p = Join-Path $startup $n
    if (Test-Path $p) {
        Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue
        Write-Output ("removed:" + $p)
    }
}
"""
    try:
        result = _run_powershell(script, timeout=10)
        for line in (result.stdout or "").splitlines():
            if line.startswith("removed:"):
                info(f"  Removed legacy Startup entry: {line.split(':', 1)[1]}")
    except (OSError, subprocess.TimeoutExpired):
        pass  # Non-fatal — the Scheduled Task is authoritative.


def _setup_autostart_windows(bin_dir: Path) -> None:
    """Register v2-daemon as a Windows Scheduled Task with auto-restart on failure.

    Parity with the Linux systemd service (Restart=on-failure, RestartSec=5) and the
    macOS launchd agent (KeepAlive=true). Runs at user logon under the current user's
    credentials (no admin required), hidden, with unlimited execution time and an
    auto-restart policy that fires if the daemon exits non-zero.

    The task supersedes the older VBS-in-Startup pattern; legacy VBS launchers are
    removed so the Scheduled Task is the only source of daemon lifecycle.
    """
    daemon_win = _windows_daemon_exe_path(bin_dir)
    # PowerShell string literal: escape single quotes by doubling them.
    daemon_ps = daemon_win.replace("'", "''")
    task_name = _WIN_TASK_NAME

    # Scheduled Task definition:
    # - AtLogon trigger for the current user (no admin needed, survives reboots)
    # - Hidden, battery-tolerant, unlimited runtime
    # - RestartCount 999 + 1-minute interval = respawn on crash, like systemd
    #   Restart=on-failure with a tiny RestartSec
    # - StartWhenAvailable catches missed windows (e.g. machine was off at logon)
    script = f"""
$ErrorActionPreference = 'Stop'
$exe = '{daemon_ps}'
if (-not (Test-Path $exe)) {{
    Write-Error ('daemon binary missing: ' + $exe)
    exit 2
}}

$action = New-ScheduledTaskAction -Execute $exe
$trigger = New-ScheduledTaskTrigger -AtLogOn -User ("$env:USERDOMAIN\\$env:USERNAME")
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -RestartCount 999 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit (New-TimeSpan -Seconds 0) `
    -MultipleInstances IgnoreNew `
    -Hidden
$principal = New-ScheduledTaskPrincipal `
    -UserId ("$env:USERDOMAIN\\$env:USERNAME") `
    -LogonType Interactive `
    -RunLevel Limited

Register-ScheduledTask `
    -TaskName '{task_name}' `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -Principal $principal `
    -Description 'AI-Foundation v2 daemon. Auto-starts at logon; auto-restarts on failure.' `
    -Force | Out-Null

Start-ScheduledTask -TaskName '{task_name}' -ErrorAction SilentlyContinue
Write-Output 'registered'
"""

    try:
        result = _run_powershell(script, timeout=20)
        if result.returncode == 0 and "registered" in (result.stdout or ""):
            _remove_legacy_startup_vbs()
            ok(f"Scheduled Task registered: {task_name}")
            info("  Auto-starts at logon, auto-restarts on crash (RestartCount=999, interval=1m).")
            return
        err_tail = (result.stderr or result.stdout or "").strip().splitlines()[-3:]
        warn("Could not register Scheduled Task: " + " | ".join(err_tail))
        info("  Run manually: installer/daemon.py --retry-autostart")
    except (OSError, subprocess.TimeoutExpired) as e:
        warn(f"Could not set up auto-start: {e}")
        info("  powershell.exe must be reachable from the installer host.")


def _setup_autostart_systemd(bin_dir: Path) -> None:
    """Create a systemd user service for the daemon."""
    service_dir = Path.home() / ".config" / "systemd" / "user"
    service_dir.mkdir(parents=True, exist_ok=True)
    service_path = service_dir / "ai-foundation-daemon.service"

    service_content = f"""[Unit]
Description=AI-Foundation v2 Daemon
After=network.target

[Service]
ExecStart={bin_dir}/v2-daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"""
    service_path.write_text(service_content)

    try:
        subprocess.run(["systemctl", "--user", "daemon-reload"], check=True, timeout=10)
        subprocess.run(["systemctl", "--user", "enable", "ai-foundation-daemon"], check=True, timeout=10)
        subprocess.run(["systemctl", "--user", "start", "ai-foundation-daemon"], timeout=10)
        ok("Systemd user service enabled and started")
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, FileNotFoundError):
        ok(f"Service file written to {service_path}")
        info("  Enable with: systemctl --user enable --now ai-foundation-daemon")


def _setup_autostart_launchd(bin_dir: Path) -> None:
    """Create a launchd plist for macOS auto-start."""
    plist_dir = Path.home() / "Library" / "LaunchAgents"
    plist_dir.mkdir(parents=True, exist_ok=True)
    plist_path = plist_dir / "com.ai-foundation.daemon.plist"

    plist_content = f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ai-foundation.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin_dir}/v2-daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"""
    plist_path.write_text(plist_content)
    try:
        subprocess.run(["launchctl", "load", str(plist_path)], timeout=10)
        ok(f"launchd agent loaded: {plist_path}")
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, FileNotFoundError):
        ok(f"Plist written to {plist_path}")
        info("  Load with: launchctl load ~/Library/LaunchAgents/com.ai-foundation.daemon.plist")
