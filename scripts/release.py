#!/usr/bin/env python3
"""
AI-Foundation Release Script
=============================
Automates cutting a new release: bumps version, syncs binaries, creates dist package.

Usage:
    python scripts/release.py patch      # 1.2.0 → 1.2.1
    python scripts/release.py minor      # 1.2.0 → 1.3.0
    python scripts/release.py major      # 1.2.0 → 2.0.0
    python scripts/release.py patch --dry-run   # Preview without changes

After running:
    1. Edit CHANGELOG.md — fill in the release notes
    2. git add -A && git commit -m "chore: v{version}"
    3. git tag v{version} && git push --tags
    4. gh release create v{version} dist/ai-foundation-v{version}.zip
"""

import argparse
import shutil
import sys
import zipfile
from datetime import date
from pathlib import Path


REPO_ROOT = Path(__file__).parent.parent
VERSION_FILE = REPO_ROOT / "version.txt"
CHANGELOG_FILE = REPO_ROOT / "CHANGELOG.md"
BIN_WINDOWS_DIR = REPO_ROOT / "bin" / "windows"
DIST_DIR = REPO_ROOT / "dist"

# Binaries to sync from ~/.ai-foundation/bin/ into bin/windows/
SYNC_BINARIES = [
    "ai-foundation-mcp.exe",
    "notebook-cli.exe",
    "teambook.exe",
    "session-start.exe",
    "v2-daemon.exe",
    "forge.exe",
    "forge-local.exe",
]

# Files/dirs included in the release zip
DIST_INCLUDES = [
    "bin/windows",
    "bin/wrappers",
    "config",
    "crates",
    "src",
    "install.py",
    "installer",
    "update.py",
    "scripts/release.py",
    "version.txt",
    "README.md",
    "QUICKSTART.md",
    "BUILDING.md",
    "CHANGELOG.md",
    "AUTOSTART.md",
    "AUDIT.md",
    "LICENSE",
    "Cargo.toml",
]


def read_version() -> tuple[int, int, int]:
    text = VERSION_FILE.read_text().strip()
    parts = text.split(".")
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        raise ValueError(f"Invalid version in version.txt: {text!r}")
    return int(parts[0]), int(parts[1]), int(parts[2])


def bump(current: tuple[int, int, int], bump_type: str) -> tuple[int, int, int]:
    major, minor, patch = current
    if bump_type == "major":
        return (major + 1, 0, 0)
    elif bump_type == "minor":
        return (major, minor + 1, 0)
    elif bump_type == "patch":
        return (major, minor, patch + 1)
    else:
        raise ValueError(f"Unknown bump type: {bump_type}")


def version_str(v: tuple[int, int, int]) -> str:
    return f"{v[0]}.{v[1]}.{v[2]}"


def sync_binaries(dry_run: bool) -> list[str]:
    """Copy updated binaries from ~/.ai-foundation/bin/ to bin/windows/."""
    import os
    home = Path(os.environ.get("USERPROFILE", Path.home()))
    source_dir = home / ".ai-foundation" / "bin"

    if not source_dir.exists():
        print(f"  ⚠  Source dir not found: {source_dir}")
        print("     Binaries in bin/windows/ will not be updated.")
        return []

    updated = []
    for name in SYNC_BINARIES:
        src = source_dir / name
        dst = BIN_WINDOWS_DIR / name
        if not src.exists():
            print(f"  -  {name}: not found in source (skipped)")
            continue

        src_size = src.stat().st_size
        dst_size = dst.stat().st_size if dst.exists() else 0

        if src_size == dst_size:
            print(f"  =  {name}: unchanged ({src_size // 1024}K)")
        else:
            change = f"{dst_size // 1024}K → {src_size // 1024}K" if dst.exists() else f"new ({src_size // 1024}K)"
            print(f"  ↑  {name}: {change}")
            if not dry_run:
                shutil.copy2(src, dst)
            updated.append(name)

    return updated


def prepend_changelog(new_version: str, dry_run: bool) -> None:
    """Add a new version section at the top of CHANGELOG.md."""
    today = date.today().strftime("%Y-%m-%d")
    new_section = f"""## v{new_version} — {today}
### Added
- <!-- describe new features here -->

### Changed
- <!-- describe changes here -->

### Fixed
- <!-- describe fixes here -->

"""
    if dry_run:
        print(f"\n  Would prepend to CHANGELOG.md:\n    ## v{new_version} — {today}")
        return

    if CHANGELOG_FILE.exists():
        existing = CHANGELOG_FILE.read_text()
        # Insert after the first heading (the document title) if present
        if existing.startswith("#"):
            # Find the first blank line after the heading
            lines = existing.split("\n")
            insert_at = 0
            for i, line in enumerate(lines):
                if i > 0 and (line.startswith("##") or (line == "" and i > 1)):
                    insert_at = i
                    break
            if insert_at:
                lines.insert(insert_at, new_section)
                CHANGELOG_FILE.write_text("\n".join(lines))
            else:
                CHANGELOG_FILE.write_text(new_section + existing)
        else:
            CHANGELOG_FILE.write_text(new_section + existing)
    else:
        CHANGELOG_FILE.write_text(f"# Changelog\n\n{new_section}")


def create_dist_zip(new_version: str, dry_run: bool) -> Path | None:
    """Create a release zip in dist/."""
    zip_name = f"ai-foundation-v{new_version}.zip"
    zip_path = DIST_DIR / zip_name

    print(f"\n  Creating dist/{zip_name}")

    if dry_run:
        total = 0
        for include in DIST_INCLUDES:
            p = REPO_ROOT / include
            if p.is_dir():
                for f in p.rglob("*"):
                    if f.is_file() and "target" not in f.parts:
                        total += f.stat().st_size
            elif p.is_file():
                total += p.stat().st_size
        print(f"  Would zip ~{total // 1024 // 1024}MB into {zip_name}")
        return None

    DIST_DIR.mkdir(exist_ok=True)
    prefix = f"ai-foundation-v{new_version}"

    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for include in DIST_INCLUDES:
            p = REPO_ROOT / include
            if not p.exists():
                continue
            if p.is_dir():
                for f in p.rglob("*"):
                    if f.is_file() and "target" not in f.parts and "__pycache__" not in f.parts:
                        arcname = f"{prefix}/{f.relative_to(REPO_ROOT)}"
                        zf.write(f, arcname)
            elif p.is_file():
                zf.write(p, f"{prefix}/{p.relative_to(REPO_ROOT)}")

    size_mb = zip_path.stat().st_size / 1024 / 1024
    print(f"  Created: {zip_path} ({size_mb:.1f}MB)")
    return zip_path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="scripts/release.py",
        description="Cut a new AI-Foundation release.",
    )
    parser.add_argument(
        "bump_type",
        choices=["patch", "minor", "major"],
        help="Version component to increment"
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Preview what would happen without making changes"
    )
    parser.add_argument(
        "--no-zip", action="store_true",
        help="Skip creating the dist/ zip package"
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    dry_run = args.dry_run

    if dry_run:
        print("DRY RUN — no files will be modified\n")

    # Read and bump version
    current = read_version()
    new = bump(current, args.bump_type)
    cur_str = version_str(current)
    new_str = version_str(new)

    print(f"Version: {cur_str} → {new_str}")
    print()

    # Sync binaries
    print("Syncing binaries from ~/.ai-foundation/bin/:")
    updated_bins = sync_binaries(dry_run)

    # Update version.txt
    if not dry_run:
        VERSION_FILE.write_text(f"{new_str}\n")
    print(f"\n  {'Would update' if dry_run else 'Updated'} version.txt → {new_str}")

    # Update CHANGELOG.md
    prepend_changelog(new_str, dry_run)

    # Create dist zip
    zip_path = None
    if not args.no_zip:
        zip_path = create_dist_zip(new_str, dry_run)

    # Print next steps
    print(f"""
{'─' * 55}
{'DRY RUN complete — no changes made.' if dry_run else 'Release prepared.'}

Next steps:
  1. Edit CHANGELOG.md — fill in release notes for v{new_str}
  2. git add -A
  3. git commit -m "chore: v{new_str}"
  4. git tag v{new_str}
  5. git push && git push --tags
  6. gh release create v{new_str} \\
       --title "v{new_str}" \\
       --notes-file <(grep -A20 '## v{new_str}' CHANGELOG.md){f' \\{chr(10)}       dist/ai-foundation-v{new_str}.zip' if zip_path else ''}
{'─' * 55}
""")

    return 0


if __name__ == "__main__":
    sys.exit(main())
