#!/usr/bin/env python3
"""
Release packaging — create distributable archives for each platform.

Usage:
    python packaging/release.py                    # Package bin/windows/ (default)
    python packaging/release.py --platform linux   # Package Linux binaries
    python packaging/release.py --all              # Package all available platforms

Creates:
    dist/ai-foundation-v{version}-{platform}-x64.{zip,tar.gz}

Also updates packaging manifests (scoop, homebrew) with correct hashes.
"""

import argparse
import json
import sys
import tarfile
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from installer.manifest import sha256_file
from installer.ui import ok, info, warn, error, step, tree_row


REPO_ROOT = Path(__file__).parent.parent
DIST_DIR = REPO_ROOT / "dist"


PLATFORM_CONFIG = {
    "windows": {
        "bin_dir": "bin/windows",
        "ext": ".exe",
        "archive": "zip",
        "arch": "x64",
    },
    "linux": {
        "bin_dir": "bin/linux",
        "ext": "",
        "archive": "tar.gz",
        "arch": "x64",
    },
    "macos-x64": {
        "bin_dir": "bin/macos-x64",
        "ext": "",
        "archive": "tar.gz",
        "arch": "x64",
    },
    "macos-aarch64": {
        "bin_dir": "bin/macos-aarch64",
        "ext": "",
        "archive": "tar.gz",
        "arch": "aarch64",
    },
}


def get_version() -> str:
    vf = REPO_ROOT / "version.txt"
    return vf.read_text().strip() if vf.exists() else "unknown"



def package_platform(platform: str, version: str) -> Path | None:
    """Create a distributable archive for one platform. Returns the archive path."""
    config = PLATFORM_CONFIG.get(platform)
    if not config:
        error(f"Unknown platform: {platform}")
        return None

    bin_dir = REPO_ROOT / config["bin_dir"]
    if not bin_dir.exists():
        warn(f"No binaries for {platform} at {bin_dir} — skipping")
        return None

    ext = config["ext"]
    archive_type = config["archive"]
    arch = config["arch"]

    DIST_DIR.mkdir(exist_ok=True)

    archive_name = f"ai-foundation-v{version}-{platform}-{arch}"
    prefix = f"ai-foundation-v{version}"

    # Collect files
    files = sorted(p for p in bin_dir.iterdir() if p.is_file())
    if not files:
        warn(f"No files found in {bin_dir}")
        return None

    if archive_type == "zip":
        archive_path = DIST_DIR / f"{archive_name}.zip"
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as zf:
            for f in files:
                arcname = f"{prefix}/bin/{f.name}"
                zf.write(f, arcname)
                info(f"  + {f.name}")
    else:
        archive_path = DIST_DIR / f"{archive_name}.tar.gz"
        with tarfile.open(archive_path, "w:gz") as tf:
            for f in files:
                arcname = f"{prefix}/{f.name}"
                tf.add(f, arcname)
                info(f"  + {f.name}")

    archive_hash = sha256_file(archive_path)
    archive_size_mb = archive_path.stat().st_size / (1024 * 1024)

    ok(f"Created: {archive_path.name} ({archive_size_mb:.1f} MB)")
    info(f"  SHA256: {archive_hash}")

    return archive_path


def update_scoop_manifest(version: str, archive_path: Path) -> None:
    """Update the Scoop manifest with the correct hash."""
    scoop_path = REPO_ROOT / "packaging" / "scoop" / "ai-foundation.json"
    if not scoop_path.exists():
        return

    archive_hash = sha256_file(archive_path)
    manifest = json.loads(scoop_path.read_text())
    manifest["version"] = version

    arch_64 = manifest.get("architecture", {}).get("64bit", {})
    arch_64["hash"] = archive_hash
    arch_64["url"] = f"https://github.com/QD25565/ai-foundation/releases/download/v{version}/{archive_path.name}"

    scoop_path.write_text(json.dumps(manifest, indent=4) + "\n")
    ok(f"Updated Scoop manifest: {scoop_path.name}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="release.py",
        description="Create distributable release archives.",
    )
    parser.add_argument(
        "--platform", choices=list(PLATFORM_CONFIG.keys()),
        help="Platform to package (default: windows)"
    )
    parser.add_argument(
        "--all", action="store_true",
        help="Package all available platforms"
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    version = get_version()

    step(f"Release Packaging v{version}")

    if args.all:
        platforms = list(PLATFORM_CONFIG.keys())
    elif args.platform:
        platforms = [args.platform]
    else:
        platforms = ["windows"]

    archives = []
    for platform in platforms:
        info(f"\nPackaging: {platform}")
        result = package_platform(platform, version)
        if result:
            archives.append((platform, result))

    if not archives:
        error("No archives created")
        return 1

    # Update package manager manifests
    print()
    step("Package Manager Manifests")
    for platform, archive_path in archives:
        if platform == "windows":
            update_scoop_manifest(version, archive_path)

    # Summary
    print()
    step("Release Summary")
    tree_row("Version", f"v{version}")
    tree_row("Archives", str(len(archives)))
    for i, (platform, archive_path) in enumerate(archives):
        is_last = i == len(archives) - 1
        size_mb = archive_path.stat().st_size / (1024 * 1024)
        tree_row(platform, f"{archive_path.name} ({size_mb:.1f} MB)", is_last=is_last)

    print()
    info("Upload archives to GitHub Releases, then update manifest URLs")

    return 0


if __name__ == "__main__":
    sys.exit(main())
