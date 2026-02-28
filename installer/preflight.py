"""
Pre-flight checks — validate system prerequisites before installation.

Catches common problems early with actionable error messages instead of
cryptic failures partway through the install.
"""

import shutil
import sys
from pathlib import Path

from .platform import Platform
from .ui import ok, warn, error, step, info


# Minimum Python version required
MIN_PYTHON = (3, 10)

# Minimum disk space required in bin_dir's filesystem (MB)
MIN_DISK_MB = 100


def check_python_version() -> bool:
    """Verify Python version meets minimum requirement."""
    current = sys.version_info[:2]
    if current >= MIN_PYTHON:
        ok(f"Python {current[0]}.{current[1]}")
        return True
    else:
        error(f"Python {current[0]}.{current[1]} — requires {MIN_PYTHON[0]}.{MIN_PYTHON[1]}+")
        info("  Install a newer Python from https://python.org")
        return False


def check_disk_space(target_dir: Path) -> bool:
    """Check that there's enough disk space for installation."""
    check_path = target_dir if target_dir.exists() else target_dir.parent
    while not check_path.exists() and check_path != check_path.parent:
        check_path = check_path.parent

    try:
        usage = shutil.disk_usage(check_path)
        free_mb = usage.free / (1024 * 1024)
        if free_mb >= MIN_DISK_MB:
            ok(f"Disk space: {free_mb:.0f} MB free")
            return True
        else:
            error(f"Disk space: {free_mb:.0f} MB free — need at least {MIN_DISK_MB} MB")
            return False
    except OSError:
        warn("Could not check disk space")
        return True  # Don't block installation on check failure


def check_bin_dir_writable(bin_dir: Path) -> bool:
    """Verify we can write to the binary directory."""
    try:
        bin_dir.mkdir(parents=True, exist_ok=True)
        test_file = bin_dir / ".write-test"
        test_file.write_text("test")
        test_file.unlink()
        ok(f"Bin dir writable: {bin_dir}")
        return True
    except OSError as e:
        error(f"Cannot write to {bin_dir}: {e}")
        info("  Check directory permissions or specify --bin-dir to use a different location")
        return False


def check_project_dir(project_dir: Path) -> bool:
    """Validate the project directory exists and is writable."""
    if not project_dir.exists():
        error(f"Project directory does not exist: {project_dir}")
        info("  Create the directory first, or specify --project with an existing path")
        return False

    if not project_dir.is_dir():
        error(f"Not a directory: {project_dir}")
        return False

    # Check writability
    try:
        test_dir = project_dir / ".claude"
        test_dir.mkdir(exist_ok=True)
        ok(f"Project dir writable: {project_dir}")
        return True
    except OSError as e:
        error(f"Cannot write to project directory: {e}")
        return False


def run_preflight(
    bin_dir: Path,
    project_dir: Path | None = None,
    platform: Platform | None = None,
) -> bool:
    """
    Run all pre-flight checks. Returns True if all critical checks pass.
    Non-critical warnings still allow installation to proceed.
    """
    step("Pre-flight Checks")
    all_ok = True

    if not check_python_version():
        all_ok = False

    if not check_disk_space(bin_dir):
        all_ok = False

    if not check_bin_dir_writable(bin_dir):
        all_ok = False

    if project_dir and not check_project_dir(project_dir):
        all_ok = False

    if all_ok:
        ok("All pre-flight checks passed")
    else:
        error("Pre-flight checks failed — fix the issues above before continuing")

    return all_ok
