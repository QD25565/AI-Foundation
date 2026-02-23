"""Binary installation — copy pre-built binaries to ~/.ai-foundation/bin/."""

import shutil
from pathlib import Path

from .platform import Platform, binary_ext, uses_windows_binaries
from .ui import ok, info, warn, step


# Core binaries needed for full functionality
CORE_BINARIES = [
    "notebook-cli",
    "teambook",
    "session-start",
    "v2-daemon",
    "ai-foundation-mcp",
]

# Optional binaries (included when present)
OPTIONAL_BINARIES = [
    "forge",
    "forge-local",
    "ai-foundation-mobile-api",
]


def get_repo_bin_dir(repo_root: Path, platform: Platform) -> Path | None:
    """Find the directory containing pre-built binaries in the repo."""
    if uses_windows_binaries(platform):
        d = repo_root / "bin" / "windows"
        if d.exists():
            return d
    # Linux/macOS: look for native binaries; not shipped in repo yet
    return None


def install(repo_root: Path, bin_dir: Path, platform: Platform, include_forge: bool = True) -> list[str]:
    """
    Copy binaries from repo into bin_dir. Returns list of installed binary names.
    Skips binaries that are already up-to-date (same size).
    """
    step("Installing binaries")
    bin_dir.mkdir(parents=True, exist_ok=True)

    source_dir = get_repo_bin_dir(repo_root, platform)
    if source_dir is None:
        warn("No pre-built binaries found for this platform. Build from source with: cargo build --release")
        return []

    ext = binary_ext(platform)
    targets = CORE_BINARIES + (OPTIONAL_BINARIES if include_forge else [])
    installed = []

    for name in targets:
        src = source_dir / f"{name}{ext}"
        if not src.exists():
            continue

        dst = bin_dir / f"{name}{ext}"
        if dst.exists() and dst.stat().st_size == src.stat().st_size:
            info(f"  Skipped: {name}{ext} (unchanged)")
            installed.append(name)
            continue

        old_size = f" ({dst.stat().st_size // 1024}K → " if dst.exists() else " ("
        shutil.copy2(src, dst)
        new_size = f"{src.stat().st_size // 1024}K)"
        ok(f"  Installed: {name}{ext}{old_size}{new_size}")
        installed.append(name)

    # Write VERSION file
    version_file = repo_root / "version.txt"
    if version_file.exists():
        version = version_file.read_text().strip()
        (bin_dir / "VERSION").write_text(version)

    return installed


def get_installed_version(bin_dir: Path) -> str | None:
    version_file = bin_dir / "VERSION"
    if version_file.exists():
        return version_file.read_text().strip()
    return None


def copy_to_project_bin(bin_dir: Path, project_dir: Path, platform: Platform) -> None:
    """Copy teambook and session-start to project/bin/ for hook access."""
    ext = binary_ext(platform)
    project_bin = project_dir / "bin"
    project_bin.mkdir(exist_ok=True)

    for name in ("teambook", "session-start"):
        src = bin_dir / f"{name}{ext}"
        if src.exists():
            shutil.copy2(src, project_bin / f"{name}{ext}")
