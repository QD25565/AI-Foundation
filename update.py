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
    python update.py --check                 # Check for remote updates without installing
    python update.py --verify                # Verify installed binaries against manifest
    python update.py --rollback              # Restore previous version
    python update.py --rollback 55           # Restore specific version
    python update.py --list-rollbacks        # Show available rollback versions
    python update.py --status                # Full installation diagnostics

Safe to re-run at any time. Skips files that haven't changed.
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from installer import platform as plat
from installer import binaries, daemon, verify
from installer import manifest as mf
from installer import remote
from installer.ui import (
    G,
    show_banner, step, ok, info, warn, error,
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
    parser.add_argument(
        "--rollback", nargs="?", const="latest", metavar="VERSION",
        help="Restore a previous version (default: most recent rollback)"
    )
    parser.add_argument(
        "--verify", action="store_true",
        help="Verify installed binaries against manifest (no changes made)"
    )
    parser.add_argument(
        "--list-rollbacks", action="store_true",
        help="List available rollback versions"
    )
    parser.add_argument(
        "--check", action="store_true",
        help="Check for updates remotely without installing"
    )
    parser.add_argument(
        "--update-url", metavar="URL",
        help="Override the remote manifest URL for update checks"
    )
    parser.add_argument(
        "--status", action="store_true",
        help="Show installation status and diagnostics"
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    available = get_version()

    show_banner(available, animated=not args.yes)

    platform = plat.detect()
    home = plat.get_home(platform)
    bin_dir = Path(args.bin_dir).expanduser().resolve() if args.bin_dir else home / ".ai-foundation" / "bin"

    # ── Route to subcommand ───────────────────────────────────────────────────
    if args.status:
        return do_status(bin_dir, platform)

    if args.list_rollbacks:
        return do_list_rollbacks(bin_dir)

    if args.verify:
        return do_verify(bin_dir)

    if args.check:
        return do_check(bin_dir, url=args.update_url)

    if args.rollback:
        target = None if args.rollback == "latest" else args.rollback
        return do_rollback(bin_dir, platform, target, yes=args.yes)

    return do_update(args, available, bin_dir, platform)


def do_status(bin_dir: Path, platform: plat.Platform) -> int:
    """Show comprehensive installation status and diagnostics."""
    from installer.platform import binary_ext

    step("Installation Status")

    ext = binary_ext(platform)

    # Version
    installed = binaries.get_installed_version(bin_dir)
    if installed:
        ok(f"Version: v{installed}")
    else:
        warn("No version file found")

    info(f"Platform: {platform.value}")
    info(f"Bin dir: {bin_dir}")
    info(f"Bin dir exists: {bin_dir.exists()}")

    if not bin_dir.exists():
        error("Binary directory does not exist — run install.py first")
        return 1

    # Installed binaries
    print()
    step("Installed Binaries")
    all_names = binaries.CORE_BINARIES + binaries.OPTIONAL_BINARIES
    found = 0
    missing_core = []
    for name in all_names:
        path = bin_dir / f"{name}{ext}"
        is_core = name in binaries.CORE_BINARIES
        if path.exists():
            size_kb = path.stat().st_size / 1024
            ok(f"  {name}{ext} ({size_kb:.0f}K)")
            found += 1
        elif is_core:
            error(f"  {name}{ext} — MISSING (core)")
            missing_core.append(name)
        else:
            info(f"  {name}{ext} — not installed (optional)")

    info(f"  {found}/{len(all_names)} binaries installed")
    if missing_core:
        warn(f"  Missing core binaries: {', '.join(missing_core)}")

    # Manifest integrity
    print()
    step("Manifest Integrity")
    manifest = mf.load(bin_dir)
    if manifest:
        info(f"Manifest version: v{manifest.get('version', '?')}")
        info(f"Channel: {manifest.get('channel', '?')}")
        info(f"Published: {manifest.get('pub_date', '?')}")

        integrity_ok, messages = mf.verify_all(bin_dir, manifest)
        for msg in messages:
            if "MISSING" in msg or "mismatch" in msg:
                error(f"  {msg}")
            elif "verified" in msg:
                ok(f"  {msg}")

        if integrity_ok:
            ok(f"All {len(messages)} binaries verified")
        else:
            error("Integrity check FAILED")
    else:
        info("No manifest found (generate with: python sign.py)")

    # Rollback availability
    print()
    step("Rollback Versions")
    versions = binaries.list_rollback_versions(bin_dir)
    if versions:
        for v in versions:
            ok(f"  v{v}")
    else:
        info("No rollback versions available")

    # Daemon status
    print()
    step("Daemon Status")
    if daemon.is_running(platform):
        ok("v2-daemon is running")
    else:
        warn("v2-daemon is NOT running")
        info(f"  Start with: {bin_dir}/v2-daemon{ext}")

    # Runtime health (skip_manifest since we already showed it above)
    print()
    verify.run_checks(bin_dir, platform, skip_manifest=True)

    return 0


def do_list_rollbacks(bin_dir: Path) -> int:
    """List available rollback versions."""
    step("Available Rollbacks")
    versions = binaries.list_rollback_versions(bin_dir)
    if not versions:
        info("No rollback versions available")
        return 0

    for i, v in enumerate(versions):
        marker = " (most recent)" if i == 0 else ""
        ok(f"  v{v}{marker}")

    print()
    info("Restore with: python update.py --rollback [VERSION]")
    return 0


def do_check(bin_dir: Path, url: str | None = None) -> int:
    """Check for updates remotely without installing anything."""
    step("Checking for Updates")

    installed = binaries.get_installed_version(bin_dir)
    if installed:
        info(f"Installed: v{installed}")
    else:
        info("No version installed")

    update_url = url or remote.get_update_url(bin_dir)
    info(f"Checking: {update_url}")
    print()

    result = remote.check_for_update(bin_dir, force=True)

    if result is None:
        ok("Already up to date")
        return 0

    remote_ver = result["remote_version"]
    channel = result["channel"]
    manifest = result["manifest"]
    binary_count = len(manifest.get("binaries", {}))
    total_size = sum(b.get("size", 0) for b in manifest.get("binaries", {}).values())

    ok(f"Update available: v{remote_ver} ({channel})")
    tree_row("Current", f"v{installed or '?'}")
    tree_row("Available", f"v{remote_ver}")
    tree_row("Channel", channel)
    tree_row("Binaries", str(binary_count))
    tree_row("Total size", f"{total_size / (1024 * 1024):.1f} MB", is_last=True)

    if manifest.get("min_daemon_version"):
        print()
        info(f"Requires daemon >= v{manifest['min_daemon_version']}")

    print()
    info("Run 'python update.py' to install this update")
    return 0


def do_verify(bin_dir: Path) -> int:
    """Verify installed binaries against their manifest."""
    step("Verifying Installation")

    manifest = mf.load(bin_dir)
    if manifest is None:
        error(f"No manifest found in {bin_dir}")
        info("Run an update first, or generate with: python sign.py")
        return 1

    installed_ver = binaries.get_installed_version(bin_dir)
    manifest_ver = manifest.get("version", "?")
    info(f"Installed version: v{installed_ver or '?'}")
    info(f"Manifest version:  v{manifest_ver}")
    info(f"Channel: {manifest.get('channel', '?')}")
    info(f"Published: {manifest.get('pub_date', '?')}")
    print()

    all_ok, messages = mf.verify_all(bin_dir, manifest)

    for msg in messages:
        if "MISSING" in msg or "mismatch" in msg:
            error(f"  {msg}")
        elif "verified" in msg:
            ok(f"  {msg}")
        else:
            info(f"  {msg}")

    print()
    if all_ok:
        ok(f"All {len(messages)} binaries verified")
    else:
        error("Verification FAILED — some binaries do not match manifest")
        info("Run 'python update.py --force' to re-install, or --rollback to restore")

    return 0 if all_ok else 1


def do_rollback(bin_dir: Path, platform: plat.Platform, target_version: str | None, yes: bool) -> int:
    """Restore binaries from a previous version."""
    versions = binaries.list_rollback_versions(bin_dir)
    if not versions:
        error("No rollback versions available")
        return 1

    if target_version and target_version not in versions:
        error(f"Rollback version '{target_version}' not found")
        info("Available versions:")
        for v in versions:
            info(f"  v{v}")
        return 1

    label = f"v{target_version}" if target_version else f"v{versions[0]}"
    current = binaries.get_installed_version(bin_dir)

    step("Rollback")
    if current:
        tree_row('Current', f'v{current}')
    tree_row('Restoring', label, is_last=True)

    if not yes:
        print()
        if not confirm(f"Roll back to {label}?", default=False):
            info("Aborted.")
            return 0

    # Stop daemon before replacing binaries
    daemon_was_running = daemon.is_running(platform)
    if daemon_was_running:
        info("Stopping daemon for rollback...")
        _stop_daemon(platform)

    success = binaries.rollback(bin_dir, platform, target_version)

    # Restart daemon
    if daemon_was_running and success:
        daemon.start(bin_dir, platform)

    if success:
        sep = G.separator(60)
        print(f'\n{sep}')
        print(f'  {G.BOLD}{G.text("✓ Rollback Complete", reverse=True)}{G.RESET}')
        print(f'{sep}\n')
    else:
        error("Rollback failed")

    return 0 if success else 1


def do_update(args: argparse.Namespace, available: str, bin_dir: Path, platform: plat.Platform) -> int:
    """Standard binary update flow."""

    # ── Version comparison ────────────────────────────────────────────────────
    step("Version Check")
    installed = binaries.get_installed_version(bin_dir)

    if installed:
        if installed == available and not args.force:
            # Even if version matches, verify manifest integrity
            manifest = mf.load(bin_dir)
            integrity_ok = True
            if manifest:
                integrity_ok, _ = mf.verify_all(bin_dir, manifest)

            ic = G.color('info')
            sc = G.color('success')
            print(f'  {ic}Installed:{G.RESET} {sc}v{installed}{G.RESET}')
            print(f'  {ic}Available:{G.RESET} {sc}v{available}{G.RESET}')
            print()

            if integrity_ok:
                ok(f"Already at v{available} — nothing to update")
                info("  Use --force to re-copy binaries anyway")
            else:
                warn(f"At v{available} but integrity check failed — forcing update")
                args.force = True

            if not args.force:
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

    # Both paths now use the manifest/rollback-aware install
    binaries.install(REPO_ROOT, bin_dir, platform, include_forge=True)

    # Write new VERSION
    (bin_dir / "VERSION").write_text(available)

    # Restart daemon if it was running
    if daemon_was_running:
        daemon.start(bin_dir, platform)

    # ── Hook scripts refresh ──────────────────────────────────────────────────
    if args.project:
        _refresh_project_hooks(REPO_ROOT, Path(args.project).expanduser().resolve())

    # ── Post-update verification ──────────────────────────────────────────────
    # verify.run_checks includes runtime health + manifest integrity
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

    # Show rollback availability
    rollbacks = binaries.list_rollback_versions(bin_dir)
    if rollbacks:
        print(f'\n  {G.text("Rollback available:")} python update.py --rollback')

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
