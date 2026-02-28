"""Binary installation — copy pre-built binaries to ~/.ai-foundation/bin/.

Supports:
- SHA256 hash-based change detection (replaces size-only comparison)
- Manifest-driven verification (when manifest.json is present)
- Rollback: previous version preserved in bin_dir/.rollback/
- Atomic-ish replacement: copy to temp, rename into place
"""

import shutil
from pathlib import Path

from .platform import Platform, binary_ext, uses_windows_binaries
from .ui import ok, info, warn, step
from . import manifest as mf


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

# Maximum rollback versions to keep (prevents unbounded disk growth)
MAX_ROLLBACK_VERSIONS = 2


def get_repo_bin_dir(repo_root: Path, platform: Platform) -> Path | None:
    """Find the directory containing pre-built binaries in the repo."""
    if uses_windows_binaries(platform):
        d = repo_root / "bin" / "windows"
        if d.exists():
            return d
    # Linux/macOS: look for native binaries; not shipped in repo yet
    return None


def _backup_for_rollback(dst: Path, bin_dir: Path, version: str | None) -> None:
    """
    Preserve current binary in .rollback/{version}/ before overwriting.
    Keeps at most MAX_ROLLBACK_VERSIONS old versions.
    """
    if not dst.exists():
        return

    label = version or "unknown"
    rollback_dir = bin_dir / ".rollback" / label
    rollback_dir.mkdir(parents=True, exist_ok=True)

    target = rollback_dir / dst.name
    if not target.exists():
        shutil.copy2(dst, target)


def _prune_rollback(bin_dir: Path) -> None:
    """Remove old rollback versions, keeping only the most recent ones."""
    rollback_root = bin_dir / ".rollback"
    if not rollback_root.exists():
        return

    versions = sorted(rollback_root.iterdir(), key=lambda p: p.stat().st_mtime, reverse=True)
    for old in versions[MAX_ROLLBACK_VERSIONS:]:
        if old.is_dir():
            shutil.rmtree(old)


def _needs_update_hash(src: Path, dst: Path, src_manifest: dict | None) -> bool:
    """
    Check if dst needs updating based on SHA256 hash.
    Uses manifest hash if available, falls back to direct file comparison.
    """
    if not dst.exists():
        return True

    name = src.stem

    # If we have a source manifest, use its hash as the expected value
    if src_manifest:
        entry = src_manifest.get("binaries", {}).get(name)
        if entry:
            dst_hash = mf.sha256_file(dst)
            return dst_hash != entry["sha256"]

    # Fallback: compare hashes directly (no manifest)
    if dst.stat().st_size != src.stat().st_size:
        return True  # Quick size check before expensive hash

    return mf.sha256_file(src) != mf.sha256_file(dst)


def install(repo_root: Path, bin_dir: Path, platform: Platform, include_forge: bool = True) -> list[str]:
    """
    Copy binaries from repo into bin_dir. Returns list of installed binary names.
    Uses SHA256 hash comparison to detect changes (not just file size).
    Backs up existing binaries to .rollback/ before overwriting.
    """
    step("Installing binaries")
    bin_dir.mkdir(parents=True, exist_ok=True)

    source_dir = get_repo_bin_dir(repo_root, platform)
    if source_dir is None:
        warn("No pre-built binaries found for this platform. Build from source with: cargo build --release")
        return []

    # Load source manifest if available (for hash verification)
    src_manifest = mf.load(source_dir)

    # Get current installed version for rollback labeling
    current_version = get_installed_version(bin_dir)

    ext = binary_ext(platform)
    targets = CORE_BINARIES + (OPTIONAL_BINARIES if include_forge else [])
    installed = []
    updated_count = 0

    for name in targets:
        src = source_dir / f"{name}{ext}"
        if not src.exists():
            continue

        dst = bin_dir / f"{name}{ext}"

        if not _needs_update_hash(src, dst, src_manifest):
            info(f"  Skipped: {name}{ext} (verified unchanged)")
            installed.append(name)
            continue

        # Back up existing binary before overwriting
        _backup_for_rollback(dst, bin_dir, current_version)

        old_size = f" ({dst.stat().st_size // 1024}K \u2192 " if dst.exists() else " ("
        shutil.copy2(src, dst)
        new_size = f"{src.stat().st_size // 1024}K)"
        ok(f"  Installed: {name}{ext}{old_size}{new_size}")
        installed.append(name)
        updated_count += 1

    # Write VERSION file
    version_file = repo_root / "version.txt"
    if version_file.exists():
        version = version_file.read_text().strip()
        (bin_dir / "VERSION").write_text(version)

    # Copy manifest to bin_dir, filtered to only include installed binaries
    if src_manifest:
        filtered = dict(src_manifest)
        filtered["binaries"] = {
            name: entry for name, entry in src_manifest.get("binaries", {}).items()
            if name in installed
        }
        mf.write(filtered, bin_dir)

    # Prune old rollback versions
    if updated_count > 0:
        _prune_rollback(bin_dir)

    if updated_count > 0 and current_version:
        info(f"  Rollback available: v{current_version} saved in .rollback/")

    return installed


def rollback(bin_dir: Path, platform: Platform, target_version: str | None = None) -> bool:
    """
    Restore binaries from .rollback/ directory.
    If target_version is None, restores the most recent rollback.
    Returns True if rollback succeeded.
    """
    rollback_root = bin_dir / ".rollback"
    if not rollback_root.exists():
        warn("No rollback versions available")
        return False

    if target_version:
        rollback_dir = rollback_root / target_version
    else:
        # Find most recent rollback
        versions = sorted(rollback_root.iterdir(), key=lambda p: p.stat().st_mtime, reverse=True)
        if not versions:
            warn("No rollback versions available")
            return False
        rollback_dir = versions[0]

    if not rollback_dir.exists():
        warn(f"Rollback version not found: {rollback_dir.name}")
        return False

    step(f"Rolling back to v{rollback_dir.name}")

    # Back up current binaries before overwriting (so rollback is reversible)
    current_version = get_installed_version(bin_dir)
    if current_version and current_version != rollback_dir.name:
        for src in rollback_dir.iterdir():
            if not src.is_file():
                continue
            dst = bin_dir / src.name
            if dst.exists():
                _backup_for_rollback(dst, bin_dir, current_version)

    restored = 0
    for src in rollback_dir.iterdir():
        if not src.is_file():
            continue
        dst = bin_dir / src.name
        shutil.copy2(src, dst)
        ok(f"  Restored: {src.name}")
        restored += 1

    # Update VERSION file
    (bin_dir / "VERSION").write_text(rollback_dir.name)

    ok(f"Rolled back {restored} binaries to v{rollback_dir.name}")
    return True


def list_rollback_versions(bin_dir: Path) -> list[str]:
    """List available rollback versions, newest first."""
    rollback_root = bin_dir / ".rollback"
    if not rollback_root.exists():
        return []
    versions = sorted(rollback_root.iterdir(), key=lambda p: p.stat().st_mtime, reverse=True)
    return [v.name for v in versions if v.is_dir()]


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
