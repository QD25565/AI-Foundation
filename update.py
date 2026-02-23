#!/usr/bin/env python3
"""
AI-Foundation Updater
=====================
Upgrades an existing AI-Foundation installation without re-running the full wizard.
Preserves your AI_ID and all configuration — only binaries and hook scripts are updated.

Usage:
    python update.py                         # Update binaries only
    python update.py --project /path/to/dir  # Also refresh hook scripts for a project
    python update.py --yes                   # Non-interactive

Safe to re-run at any time. Skips files that haven't changed.
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from installer import platform as plat
from installer import binaries, daemon, verify
from installer.ui import (
    G,
    show_banner, step, ok, info, warn,
    tree_row, confirm, pause,
)


REPO_ROOT = Path(__file__).parent
VERSION_FILE = REPO_ROOT / "version.txt"


def get_version() -> str:
    return VERSION_FILE.read_text().strip() if VERSION_FILE.exists() else "unknown"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="update.py",
        description="Update AI-Foundation binaries and hook scripts.",
    )
    parser.add_argument(
        "--project", metavar="PATH",
        help="Project directory whose hook scripts should be refreshed"
    )
    parser.add_argument(
        "--bin-dir", metavar="PATH",
        help="Binary installation directory (default: ~/.ai-foundation/bin/)"
    )
    parser.add_argument(
        "--yes", "-y", action="store_true",
        help="Non-interactive: no prompts"
    )
    parser.add_argument(
        "--force", action="store_true",
        help="Re-copy binaries even if they appear unchanged"
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    available = get_version()

    show_banner(available, animated=not args.yes)

    platform = plat.detect()
    home = plat.get_home(platform)
    bin_dir = Path(args.bin_dir).expanduser().resolve() if args.bin_dir else home / ".ai-foundation" / "bin"

    # ── Version comparison ────────────────────────────────────────────────────
    step("Version Check")
    installed = binaries.get_installed_version(bin_dir)

    if installed:
        if installed == available and not args.force:
            ic = G.color('info')
            sc = G.color('success')
            print(f'  {ic}Installed:{G.RESET} {sc}v{installed}{G.RESET}')
            print(f'  {ic}Available:{G.RESET} {sc}v{available}{G.RESET}')
            print()
            ok(f"Already at v{available} — nothing to update")
            info("  Use --force to re-copy binaries anyway")
            if not args.yes:
                pause("Press Enter to exit...")
            return 0
        tree_row('Installed', f'v{installed}')
        tree_row('Available', f'v{available}', is_last=True)
    else:
        info(f"No existing installation found at {bin_dir}")
        info(f"Installing v{available}")

    if not args.yes:
        print()
        if not confirm("Proceed with update?", default=True):
            info("Aborted.")
            return 0

    # ── Binary update ─────────────────────────────────────────────────────────
    step("Updating Binaries")

    # Stop daemon before replacing its binary
    daemon_was_running = daemon.is_running(platform)
    if daemon_was_running:
        info("Stopping daemon for binary replacement...")
        _stop_daemon(platform)

    if args.force:
        _force_update_binaries(REPO_ROOT, bin_dir, platform)
    else:
        binaries.install(REPO_ROOT, bin_dir, platform, include_forge=True)

    # Write new VERSION
    (bin_dir / "VERSION").write_text(available)

    # Restart daemon if it was running
    if daemon_was_running:
        daemon.start(bin_dir, platform)

    # ── Hook scripts refresh ──────────────────────────────────────────────────
    if args.project:
        _refresh_project_hooks(REPO_ROOT, Path(args.project).expanduser().resolve())

    # ── Verify ────────────────────────────────────────────────────────────────
    print()
    verify.run_checks(bin_dir, platform)

    # ── Summary ───────────────────────────────────────────────────────────────
    sep = G.separator(60)
    print(f'\n{sep}')
    print(f'  {G.BOLD}{G.text("✓ Update Complete!", reverse=True)}{G.RESET}')
    print(f'  {G.text(f"AI-Foundation v{available}")}')
    print(f'{sep}\n')

    v_from = f'v{installed} → v{available}' if installed and installed != available else f'v{available}'
    tree_row('Version',  v_from)
    tree_row('Binaries', str(bin_dir))
    if args.project:
        tree_row('Hooks', args.project, is_last=True)
    else:
        tree_row('AI_ID', 'unchanged', is_last=True)

    print(f'\n  {G.text("Your AI_ID and configuration were not changed")}')

    if not args.yes:
        pause()
    return 0


def _stop_daemon(platform: plat.Platform) -> None:
    """Best-effort daemon stop before binary replacement."""
    import subprocess
    from installer.platform import binary_ext
    ext = binary_ext(platform)
    try:
        if platform == plat.Platform.WINDOWS:
            subprocess.run(["taskkill", "/f", "/im", f"v2-daemon{ext}"],
                           capture_output=True, timeout=5)
        else:
            subprocess.run(["pkill", "-f", "v2-daemon"],
                           capture_output=True, timeout=5)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass


def _force_update_binaries(repo_root: Path, bin_dir: Path, platform: plat.Platform) -> None:
    """Force-copy all binaries regardless of size."""
    import shutil
    from installer.platform import binary_ext, uses_windows_binaries
    from installer.binaries import CORE_BINARIES, OPTIONAL_BINARIES

    source_dir = repo_root / "bin" / "windows" if uses_windows_binaries(platform) else None
    if not source_dir or not source_dir.exists():
        warn("No pre-built binaries found for forced update")
        return

    ext = binary_ext(platform)
    for name in CORE_BINARIES + OPTIONAL_BINARIES:
        src = source_dir / f"{name}{ext}"
        if src.exists():
            shutil.copy2(src, bin_dir / f"{name}{ext}")
            ok(f"  Updated: {name}{ext}")


def _refresh_project_hooks(repo_root: Path, project_dir: Path) -> None:
    """Re-copy hook scripts to a project directory. Never touches settings.json."""
    import shutil

    step(f"Refreshing Hooks: {project_dir.name}")
    config_claude = repo_root / "config" / "claude"
    claude_dir = project_dir / ".claude"
    hooks_dir = claude_dir / "hooks"

    if not claude_dir.exists():
        warn(f"No .claude/ found in {project_dir} — skipping hook refresh")
        return

    hooks_dir.mkdir(exist_ok=True)
    for src, dst_name in [
        (config_claude / "mcp-launcher.py",            "mcp-launcher.py"),
        (config_claude / "hooks" / "SessionStart.py",  "hooks/SessionStart.py"),
        (config_claude / "hooks" / "platform_utils.py","hooks/platform_utils.py"),
    ]:
        if src.exists():
            shutil.copy2(src, claude_dir / dst_name)
            ok(f"  Updated: .claude/{dst_name}")


if __name__ == "__main__":
    sys.exit(main())
