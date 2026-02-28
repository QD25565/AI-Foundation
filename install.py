#!/usr/bin/env python3
"""
AI-Foundation Installer
=======================
Sets up AI-Foundation for a Claude Code project directory.

Usage:
    python install.py                                    # Interactive wizard
    python install.py --project /path/to/project        # Specify project dir
    python install.py --project /path/to/project --yes  # Non-interactive
    python install.py --uninstall                        # Remove installation

Run `python install.py --help` for all options.
"""

import argparse
import os
import sys
from pathlib import Path

# Ensure the installer package is importable from this script's directory
sys.path.insert(0, str(Path(__file__).parent))

from installer import platform as plat
from installer import binaries, daemon, forge, project, shell, verify, preflight
from installer.ui import (
    G,
    show_banner, step, header, ok, info, warn, error,
    tree_row, prompt, confirm, pause,
)


REPO_ROOT = Path(__file__).parent
VERSION_FILE = REPO_ROOT / "version.txt"


def get_version() -> str:
    if VERSION_FILE.exists():
        return VERSION_FILE.read_text().strip()
    return "unknown"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="install.py",
        description="AI-Foundation installer — sets up memory, coordination, and MCP tools.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python install.py
  python install.py --project ~/my-claude-project --ai-id nova-312
  python install.py --project ~/my-claude-project --yes --no-forge
  python install.py --uninstall --keep-data
        """
    )
    parser.add_argument(
        "--project", metavar="PATH",
        help="Claude Code project directory to configure (default: current directory)"
    )
    parser.add_argument(
        "--ai-id", metavar="AI_ID",
        help="AI identity string (e.g. nova-312). Generated randomly if not provided."
    )
    parser.add_argument(
        "--bin-dir", metavar="PATH",
        help="Directory to install binaries (default: ~/.ai-foundation/bin/)"
    )
    parser.add_argument(
        "--yes", "-y", action="store_true",
        help="Non-interactive: accept all defaults without prompting"
    )
    parser.add_argument(
        "--no-forge", action="store_true",
        help="Skip Forge CLI setup"
    )
    parser.add_argument(
        "--no-autostart", action="store_true",
        help="Skip daemon auto-start configuration"
    )
    parser.add_argument(
        "--uninstall", action="store_true",
        help="Remove AI-Foundation installation"
    )
    parser.add_argument(
        "--keep-data", action="store_true",
        help="With --uninstall: keep notebook data (~/.ai-foundation/agents/)"
    )
    return parser.parse_args()


def resolve_project_dir(args: argparse.Namespace, yes: bool) -> Path:
    if args.project:
        return Path(args.project).expanduser().resolve()
    if yes:
        return Path.cwd()
    chosen = prompt("Project directory to configure", str(Path.cwd()))
    return Path(chosen).expanduser().resolve()


def resolve_bin_dir(args: argparse.Namespace, home: Path) -> Path:
    if args.bin_dir:
        return Path(args.bin_dir).expanduser().resolve()
    return home / ".ai-foundation" / "bin"


def resolve_ai_id(args: argparse.Namespace, project_dir: Path, yes: bool) -> str:
    # Priority: --ai-id flag > existing settings.json > generate new
    if args.ai_id:
        return args.ai_id

    existing = project.read_existing_ai_id(project_dir)
    if existing:
        if yes:
            return existing
        if confirm(f"Found existing AI_ID '{existing}'. Keep it?", default=True):
            return existing

    new_id = project.generate_ai_id()
    if yes:
        return new_id
    chosen = prompt("AI identity (e.g. nova-312)", new_id)
    return chosen if chosen else new_id


def do_install(args: argparse.Namespace) -> int:
    version = get_version()
    yes = args.yes

    show_banner(version, animated=not yes)

    platform = plat.detect()
    home = plat.get_home(platform)
    project_dir = resolve_project_dir(args, yes)
    bin_dir = resolve_bin_dir(args, home)

    step("Configuration")
    info(f"Platform: {platform.value}")
    info(f"Home:     {home}")
    info(f"Bin dir:  {bin_dir}")
    info(f"Project:  {project_dir}")

    # Pre-flight checks
    if not preflight.run_preflight(bin_dir, project_dir, platform):
        error("Fix the issues above before continuing.")
        return 1

    if not yes:
        print()
        if not confirm("Proceed with installation?", default=True):
            info("Aborted.")
            return 0

    # 1. Install binaries (step() called inside binaries.install)
    include_forge = not args.no_forge
    installed = binaries.install(REPO_ROOT, bin_dir, platform, include_forge=include_forge)
    if not installed:
        error("No binaries were installed. Check that bin/windows/ exists in the repo.")
        return 1
    ok(f"Installed {len(installed)} binaries to {bin_dir}")

    # 1b. Add bin dir to shell PATH (Linux/WSL/macOS only)
    step("Shell PATH setup")
    shell.setup_path(platform, home)

    # 2. Start daemon (step() called inside daemon.start)
    daemon.start(bin_dir, platform)

    # 3. Auto-start setup (optional)
    if not args.no_autostart:
        do_autostart = yes or confirm("Set up daemon auto-start on login?", default=True)
        if do_autostart:
            daemon.setup_autostart(bin_dir, platform, yes=yes)

    # 4. Determine AI_ID
    ai_id = resolve_ai_id(args, project_dir, yes)

    # 5. Configure project directory (step() called inside project.configure)
    project.configure(REPO_ROOT, project_dir, bin_dir, platform, ai_id)

    # 6. Forge setup (optional)
    if not args.no_forge:
        forge.configure(REPO_ROOT, home, ai_id)

    # 7. Verify (step() called inside verify.run_checks)
    print()
    all_good = verify.run_checks(bin_dir, platform)

    # 8. Summary
    _print_summary(version, project_dir, bin_dir, ai_id, all_good, interactive=not yes)

    return 0 if all_good else 1


def do_uninstall(args: argparse.Namespace) -> int:
    import shutil

    version = get_version()
    show_banner(version, animated=False)

    platform = plat.detect()
    home = plat.get_home(platform)
    bin_dir = resolve_bin_dir(args, home)

    step("Uninstall")
    warn("This will remove AI-Foundation binaries and configuration.")
    if not args.keep_data:
        warn("Notebook data (~/.ai-foundation/agents/) will also be removed.")
        warn("Use --keep-data to preserve your notes.")

    if not args.yes:
        print()
        if not confirm("Continue with uninstall?", default=False):
            info("Aborted.")
            return 0

    af_dir = home / ".ai-foundation"
    removed = []

    if bin_dir.exists():
        shutil.rmtree(bin_dir)
        removed.append(str(bin_dir))

    agents_dir = af_dir / "agents"
    if not args.keep_data and agents_dir.exists():
        shutil.rmtree(agents_dir)
        removed.append(str(agents_dir))

    forge_dir = home / ".forge"
    if forge_dir.exists():
        if args.yes or confirm(f"Remove Forge config at {forge_dir}?", default=False):
            shutil.rmtree(forge_dir)
            removed.append(str(forge_dir))

    _remove_autostart(platform, home)

    # Summary box
    sep = G.separator(60)
    print(f'\n{sep}')
    print(f'  {G.BOLD}{G.text("✓ Uninstall Complete", reverse=True)}{G.RESET}')
    print(f'{sep}\n')
    for r in removed:
        ok(f"Removed: {r}")
    if args.keep_data:
        info(f"Kept: {agents_dir} (notebook data)")
    info("Project .claude/ directories were not modified.")

    if not args.yes:
        pause()
    return 0


def _remove_autostart(platform: plat.Platform, home: Path) -> None:
    import subprocess

    if platform in (plat.Platform.WINDOWS, plat.Platform.WSL):
        try:
            result = subprocess.run(
                ["cmd.exe", "/c", "echo %APPDATA%"],
                capture_output=True, text=True, timeout=5
            )
            app_data = result.stdout.strip()
            try:
                wsl_result = subprocess.run(
                    ["wslpath", app_data], capture_output=True, text=True, timeout=5
                )
                startup = Path(wsl_result.stdout.strip()) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Startup"
            except Exception:
                startup = Path(app_data) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Startup"
            vbs = startup / "ai-foundation-daemon.vbs"
            if vbs.exists():
                vbs.unlink()
                ok("Removed: Windows auto-start entry")
        except Exception:
            pass

    elif platform == plat.Platform.LINUX:
        service = home / ".config" / "systemd" / "user" / "ai-foundation-daemon.service"
        if service.exists():
            try:
                import subprocess
                subprocess.run(["systemctl", "--user", "disable", "--now", "ai-foundation-daemon"], timeout=10)
            except Exception:
                pass
            service.unlink()
            ok("Removed: systemd user service")

    elif platform == plat.Platform.MACOS:
        plist = home / "Library" / "LaunchAgents" / "com.ai-foundation.daemon.plist"
        if plist.exists():
            try:
                import subprocess
                subprocess.run(["launchctl", "unload", str(plist)], timeout=10)
            except Exception:
                pass
            plist.unlink()
            ok("Removed: launchd agent")


def _print_summary(
    version: str,
    project_dir: Path,
    bin_dir: Path,
    ai_id: str,
    success: bool,
    interactive: bool,
) -> None:
    sep = G.separator(60)
    print(f'\n{sep}')
    if success:
        print(f'  {G.BOLD}{G.text("✓ Installation Complete!", reverse=True)}{G.RESET}')
        print(f'  {G.text("All components installed successfully")}')
    else:
        print(f'  {G.BOLD}{G.text("⚠ Installation complete with warnings", reverse=True)}{G.RESET}')
        print(f'  {G.text("Some checks failed — see above for details")}')
    print(f'{sep}\n')

    print(f'  {G.text("Installation Summary:")}')
    tree_row('Version',  f'v{version}')
    tree_row('AI_ID',    ai_id)
    tree_row('Binaries', str(bin_dir))
    tree_row('Project',  str(project_dir), is_last=True)

    ic = G.color('info')
    print(f'\n  {G.text("Next Steps:")}')
    print(f'    {ic}1.{G.RESET} {G.text(f"Start Claude Code in {project_dir}")}')
    print(f'    {ic}2.{G.RESET} {G.text("Session hook is active — Notebook and Teambook are ready")}')
    print(f'    {ic}3.{G.RESET} {G.text("Try: teambook_status in Claude Code to verify MCP connection")}')
    print(f'    {ic}4.{G.RESET} {G.text("Add API keys to ~/.forge/config.toml to use Forge CLI")}')
    print(f'\n  {G.text("Docs: https://github.com/QD25565/ai-foundation")}')

    if not success:
        print(f'\n  {G.text("Daemon not running? Start it manually:")}')
        print(f'    {G.color("info")}{bin_dir}/v2-daemon(.exe){G.RESET}')
        print(f'    {G.text("Then re-run: python install.py --yes")}')

    if interactive:
        pause()


def main() -> int:
    args = parse_args()
    try:
        if args.uninstall:
            return do_uninstall(args)
        else:
            return do_install(args)
    except KeyboardInterrupt:
        print()
        info("Interrupted.")
        return 130


if __name__ == "__main__":
    sys.exit(main())
