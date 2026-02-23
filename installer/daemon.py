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


def _setup_autostart_windows(bin_dir: Path) -> None:
    """Create a VBS launcher in the Windows Startup folder."""
    try:
        result = subprocess.run(
            ["cmd.exe", "/c", "echo %APPDATA%"],
            capture_output=True, text=True, timeout=5
        )
        app_data = result.stdout.strip()
        if not app_data or app_data == "%APPDATA%":
            warn("Could not locate Windows APPDATA — skipping auto-start setup")
            return

        # Resolve to WSL path if needed
        try:
            wsl_result = subprocess.run(
                ["wslpath", app_data], capture_output=True, text=True, timeout=5
            )
            startup_dir = Path(wsl_result.stdout.strip()) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Startup"
        except (FileNotFoundError, subprocess.TimeoutExpired):
            startup_dir = Path(app_data) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Startup"

        vbs_path = startup_dir / "ai-foundation-daemon.vbs"
        daemon_exe = bin_dir / "v2-daemon.exe"

        vbs_content = f'''Set objShell = CreateObject("WScript.Shell")
objShell.Run Chr(34) & "{daemon_exe}" & Chr(34), 0, False
'''
        vbs_path.write_text(vbs_content)
        ok(f"Auto-start configured: {vbs_path}")
        info("  Daemon will start automatically on Windows login.")

    except (OSError, subprocess.TimeoutExpired) as e:
        warn(f"Could not set up auto-start: {e}")
        info("  Run bin/setup-autostart.ps1 manually for Windows auto-start.")


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
