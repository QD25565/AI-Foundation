"""Platform detection and home directory resolution."""

import os
import subprocess
import sys
from enum import Enum
from pathlib import Path


class Platform(Enum):
    WINDOWS = "windows"
    WSL = "wsl"
    LINUX = "linux"
    MACOS = "macos"


def detect() -> Platform:
    if sys.platform == "win32":
        return Platform.WINDOWS
    if sys.platform == "darwin":
        return Platform.MACOS
    # Linux or WSL
    try:
        version = Path("/proc/version").read_text().lower()
        if "microsoft" in version or "wsl" in version:
            return Platform.WSL
    except OSError:
        pass
    return Platform.LINUX


def get_windows_home(platform: Platform) -> Path | None:
    """Return the Windows user home as a WSL-accessible path, or None on failure."""
    if platform == Platform.WSL:
        try:
            result = subprocess.run(
                ["cmd.exe", "/c", "echo %USERPROFILE%"],
                capture_output=True, text=True, timeout=5
            )
            win_path = result.stdout.strip()
            if win_path and win_path != "%USERPROFILE%":
                wsl_result = subprocess.run(
                    ["wslpath", win_path],
                    capture_output=True, text=True, timeout=5
                )
                p = Path(wsl_result.stdout.strip())
                if p.exists():
                    return p
        except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
            pass
    return None


def get_home(platform: Platform) -> Path:
    """Return the effective home directory for binary installation."""
    if platform == Platform.WINDOWS:
        return Path(os.environ.get("USERPROFILE", str(Path.home())))
    elif platform == Platform.WSL:
        win_home = get_windows_home(platform)
        if win_home:
            return win_home
        return Path.home()
    else:
        return Path.home()


def binary_ext(platform: Platform) -> str:
    """Return '.exe' on Windows/WSL, '' otherwise."""
    return ".exe" if platform in (Platform.WINDOWS, Platform.WSL) else ""


def uses_windows_binaries(platform: Platform) -> bool:
    return platform in (Platform.WINDOWS, Platform.WSL)


def python_cmd() -> str:
    """Return the Python command available on this system."""
    for cmd in ("python3", "python"):
        try:
            result = subprocess.run(
                [cmd, "--version"], capture_output=True, text=True, timeout=3
            )
            if result.returncode == 0:
                return cmd
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
    return "python3"  # fallback
