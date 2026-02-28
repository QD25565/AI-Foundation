"""
Remote update checking — fetch and compare manifests from an update server.

Supports:
- Checking for newer versions via remote manifest.json
- Downloading new binaries when updates are available
- Configurable update URL (defaults to GitHub Releases)
- Respects HTTP 204 No Content (already up-to-date, Tauri convention)
- Caches last-check timestamp to avoid hammering the server

The remote manifest has the same format as the local one (see manifest.py).
An update is available when remote version > installed version.
"""

import json
import time
from pathlib import Path
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen

from . import manifest as mf
from .ui import ok, info, warn, error


# Default update endpoint — override via update-url.txt in bin_dir or --update-url flag.
DEFAULT_UPDATE_URL = "https://github.com/QD25565/ai-foundation/releases/latest/download/manifest.json"

# Minimum seconds between remote checks (15 minutes)
CHECK_INTERVAL = 900

# Cache file for last-check metadata
_CACHE_FILENAME = ".update-check"

# HTTP timeout for remote requests (seconds)
_HTTP_TIMEOUT = 15


def get_update_url(bin_dir: Path) -> str:
    """Read update URL from config, falling back to default."""
    config = bin_dir / "update-url.txt"
    if config.exists():
        url = config.read_text().strip()
        if url:
            return url
    return DEFAULT_UPDATE_URL


def _read_cache(bin_dir: Path) -> dict[str, Any]:
    """Read last-check cache. Returns empty dict if missing/corrupt."""
    cache_path = bin_dir / _CACHE_FILENAME
    if not cache_path.exists():
        return {}
    try:
        return json.loads(cache_path.read_text())
    except (json.JSONDecodeError, OSError):
        return {}


def _write_cache(bin_dir: Path, data: dict[str, Any]) -> None:
    """Write last-check cache."""
    cache_path = bin_dir / _CACHE_FILENAME
    try:
        cache_path.write_text(json.dumps(data) + "\n")
    except OSError:
        pass  # Non-critical — worst case we check again next time


def should_check(bin_dir: Path) -> bool:
    """Return True if enough time has passed since the last remote check."""
    cache = _read_cache(bin_dir)
    last_check = cache.get("last_check", 0)
    return (time.time() - last_check) >= CHECK_INTERVAL


def fetch_remote_manifest(url: str | None = None, bin_dir: Path | None = None) -> dict[str, Any] | None:
    """
    Fetch the remote manifest from the update server.
    Returns the parsed manifest dict, or None on failure.

    HTTP 204 means "already up-to-date" (Tauri convention) — returns None.
    """
    if url is None:
        if bin_dir is not None:
            url = get_update_url(bin_dir)
        else:
            url = DEFAULT_UPDATE_URL

    if not url:
        return None  # No update URL configured

    req = Request(url, headers={
        "User-Agent": "AI-Foundation-Updater/1.0",
        "Accept": "application/json",
    })

    try:
        with urlopen(req, timeout=_HTTP_TIMEOUT) as resp:
            if resp.status == 204:
                return None  # No update available (Tauri convention)

            data = resp.read()
            manifest = json.loads(data)

            # Basic validation — must have version and binaries
            if "version" not in manifest or "binaries" not in manifest:
                warn("Remote manifest missing required fields")
                return None

            return manifest

    except URLError as e:
        # Network errors are expected when offline — don't alarm the user
        info(f"Could not reach update server: {e.reason}")
        return None
    except json.JSONDecodeError:
        warn("Remote manifest is not valid JSON")
        return None
    except OSError as e:
        info(f"Update check failed: {e}")
        return None


def _version_newer(remote: str, installed: str) -> bool:
    """
    Compare version strings. Supports numeric versions (e.g. "58" > "57")
    and semver-like versions (e.g. "1.2.3" > "1.2.2").
    Returns True if remote is strictly newer than installed.
    """
    try:
        # Try simple integer comparison first (our primary version format)
        return int(remote) > int(installed)
    except ValueError:
        pass

    # Fall back to tuple comparison for dotted versions
    try:
        r_parts = tuple(int(x) for x in remote.split("."))
        i_parts = tuple(int(x) for x in installed.split("."))
        return r_parts > i_parts
    except ValueError:
        # Last resort: string comparison
        return remote > installed


def check_for_update(bin_dir: Path, force: bool = False) -> dict[str, Any] | None:
    """
    Check if a newer version is available remotely.

    Returns a dict with update info if available:
        {"remote_version": "59", "installed_version": "58", "manifest": {...}, "channel": "stable"}
    Returns None if already up-to-date or check failed.

    Respects CHECK_INTERVAL unless force=True.
    """
    if not force and not should_check(bin_dir):
        return None

    installed_version = None
    version_file = bin_dir / "VERSION"
    if version_file.exists():
        installed_version = version_file.read_text().strip()

    remote = fetch_remote_manifest(bin_dir=bin_dir)

    # Record that we checked, regardless of outcome
    _write_cache(bin_dir, {
        "last_check": time.time(),
        "remote_version": remote.get("version") if remote else None,
        "had_update": False,
    })

    if remote is None:
        return None

    remote_version = remote.get("version", "")
    channel = remote.get("channel", "stable")

    if not installed_version:
        # No installed version — any remote version is an update
        _write_cache(bin_dir, {
            "last_check": time.time(),
            "remote_version": remote_version,
            "had_update": True,
        })
        return {
            "remote_version": remote_version,
            "installed_version": None,
            "manifest": remote,
            "channel": channel,
        }

    if _version_newer(remote_version, installed_version):
        _write_cache(bin_dir, {
            "last_check": time.time(),
            "remote_version": remote_version,
            "had_update": True,
        })
        return {
            "remote_version": remote_version,
            "installed_version": installed_version,
            "manifest": remote,
            "channel": channel,
        }

    return None  # Up to date


def download_binary(url: str, dest: Path) -> bool:
    """
    Download a single binary from a URL to a local path.
    Returns True on success.
    """
    req = Request(url, headers={
        "User-Agent": "AI-Foundation-Updater/1.0",
    })

    try:
        with urlopen(req, timeout=60) as resp:
            data = resp.read()

        # Write to temp file first, then rename (atomic-ish)
        tmp = dest.with_suffix(dest.suffix + ".tmp")
        tmp.write_bytes(data)
        tmp.rename(dest)
        return True

    except (URLError, OSError) as e:
        error(f"Download failed: {e}")
        if dest.with_suffix(dest.suffix + ".tmp").exists():
            dest.with_suffix(dest.suffix + ".tmp").unlink()
        return False


def verify_download(path: Path, expected_hash: str, expected_size: int) -> bool:
    """Verify a downloaded binary against expected hash and size."""
    if not path.exists():
        return False

    if path.stat().st_size != expected_size:
        error(f"Size mismatch for {path.name}: got {path.stat().st_size}, expected {expected_size}")
        return False

    actual_hash = mf.sha256_file(path)
    if actual_hash != expected_hash:
        error(f"SHA256 mismatch for {path.name}")
        return False

    return True
