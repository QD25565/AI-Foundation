"""
Release manifest — SHA256 integrity verification for AI-Foundation binaries.

The manifest is a JSON file listing every binary with its SHA256 hash and size.
At install/update time, binaries are verified against the manifest to detect
corruption, tampering, or incomplete downloads.

Manifest format:
{
    "version": "58",
    "channel": "stable",
    "pub_date": "2026-02-27T10:51:00Z",
    "min_daemon_version": "57",
    "binaries": {
        "teambook": {
            "sha256": "a1b2c3...",
            "size": 3481600
        },
        ...
    }
}
"""

import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MANIFEST_FILENAME = "manifest.json"


def sha256_file(path: Path) -> str:
    """Compute SHA256 hex digest of a file. Reads in 64KB chunks."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def generate(
    bin_dir: Path,
    version: str,
    channel: str = "stable",
    min_daemon_version: str | None = None,
) -> dict[str, Any]:
    """
    Generate a release manifest from a directory of binaries.
    Scans bin_dir for .exe files (Windows) or executable files and records
    their SHA256 hash and size.
    """
    binaries: dict[str, dict[str, Any]] = {}

    for path in sorted(bin_dir.iterdir()):
        if not path.is_file():
            continue
        # Skip non-binary files
        if path.suffix not in ("", ".exe"):
            continue
        # Skip manifest itself
        if path.name == MANIFEST_FILENAME:
            continue
        # Skip VERSION file and other metadata
        if path.name in ("VERSION", "config.toml"):
            continue

        name = path.stem  # "teambook" from "teambook.exe"
        binaries[name] = {
            "sha256": sha256_file(path),
            "size": path.stat().st_size,
        }

    manifest = {
        "version": version,
        "channel": channel,
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "binaries": binaries,
    }

    if min_daemon_version:
        manifest["min_daemon_version"] = min_daemon_version

    return manifest


def write(manifest: dict[str, Any], dest: Path) -> Path:
    """Write manifest to a JSON file. Returns the path written."""
    out = dest / MANIFEST_FILENAME if dest.is_dir() else dest
    out.write_text(json.dumps(manifest, indent=2) + "\n")
    return out


def load(source: Path) -> dict[str, Any] | None:
    """Load a manifest from a file or directory. Returns None if not found."""
    path = source / MANIFEST_FILENAME if source.is_dir() else source
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None


def verify_binary(
    binary_path: Path,
    manifest: dict[str, Any],
) -> tuple[bool, str]:
    """
    Verify a single binary against the manifest.
    Returns (passed, message).
    """
    name = binary_path.stem
    entry = manifest.get("binaries", {}).get(name)

    if entry is None:
        return False, f"{name}: not in manifest"

    # Size check first (fast)
    actual_size = binary_path.stat().st_size
    expected_size = entry.get("size", 0)
    if actual_size != expected_size:
        return False, f"{name}: size mismatch ({actual_size} != {expected_size})"

    # SHA256 check (thorough)
    actual_hash = sha256_file(binary_path)
    expected_hash = entry.get("sha256", "")
    if actual_hash != expected_hash:
        return False, f"{name}: SHA256 mismatch"

    return True, f"{name}: verified"


def verify_all(
    bin_dir: Path,
    manifest: dict[str, Any],
) -> tuple[bool, list[str]]:
    """
    Verify all binaries in bin_dir against the manifest.
    Returns (all_passed, list of messages).
    """
    messages: list[str] = []
    all_ok = True

    for name, entry in manifest.get("binaries", {}).items():
        # Try with and without .exe extension
        path = bin_dir / name
        if not path.exists():
            path = bin_dir / f"{name}.exe"
        if not path.exists():
            messages.append(f"{name}: MISSING")
            all_ok = False
            continue

        passed, msg = verify_binary(path, manifest)
        messages.append(msg)
        if not passed:
            all_ok = False

    return all_ok, messages


def needs_update(
    binary_path: Path,
    manifest: dict[str, Any],
) -> bool:
    """Check if a binary needs updating based on the manifest hash."""
    name = binary_path.stem
    entry = manifest.get("binaries", {}).get(name)
    if entry is None:
        return False  # Not in manifest, don't touch it

    if not binary_path.exists():
        return True  # Missing, needs install

    # Quick size check
    if binary_path.stat().st_size != entry.get("size", 0):
        return True

    # Full hash check
    return sha256_file(binary_path) != entry.get("sha256", "")
