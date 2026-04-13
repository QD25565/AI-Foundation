"""Shell wrapper installation — AI_ID-aware wrappers around Windows .exe binaries.

Problem this solves:
  When an AI invokes `teambook` or `notebook` from a WSL shell, the .exe does
  not inherit the AI_ID environment variable unless WSLENV exports it. Without
  AI_ID the binary falls back to daemon-resolved identity, which is wrong when
  multiple instances share one daemon or when AI_ID is pinned in the project's
  .claude/settings.json.

Fix:
  Each CLI gets a tiny `/bin/sh` wrapper that
    1. sources _resolve_ai_id.sh (walks CWD → .claude/settings.json, instances.toml, ~/.claude/settings.json)
    2. appends AI_ID to WSLENV so the .exe sees it
    3. execs the real .exe with the original argv

Only installed on WSL. Native Windows doesn't need them (no bash). Native
Linux/macOS doesn't need them (no .exe; native binaries work directly).
"""

import os
import shutil
import stat
from pathlib import Path

from .platform import Platform
from .ui import ok, info, step, warn


# CLIs that benefit from AI_ID resolution. Match binaries actually shipped
# (see binaries.CORE_BINARIES and OPTIONAL_BINARIES) plus a few convenience
# aliases like `notebook` (maps to notebook-cli.exe — historical naming).
WRAPPER_BINARIES = [
    # name,           exe_target
    ("teambook",      "teambook.exe"),
    ("notebook",      "notebook-cli.exe"),
    ("notebook-cli",  "notebook-cli.exe"),
    ("forge",         "forge.exe"),
    ("forge-local",   "forge-local.exe"),
    ("session-start", "session-start.exe"),
]


_WRAPPER_TEMPLATE = """#!/bin/sh
# {name} — AI_ID-aware wrapper for {exe}
# Resolves AI_ID from .claude/settings.json or instances.toml when not in env.
_BIN_DIR="$(dirname "$0")"
. "$_BIN_DIR/_resolve_ai_id.sh"
export WSLENV="${{WSLENV:+$WSLENV:}}AI_ID"
exec "$_BIN_DIR/{exe}" "$@"
"""


def _wrappers_source_dir(repo_root: Path) -> Path:
    return repo_root / "bin" / "wrappers"


def _make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def install(repo_root: Path, bin_dir: Path, platform: Platform) -> list[str]:
    """Install AI_ID resolver + per-binary wrappers into bin_dir.

    Returns the list of wrapper names written. Returns [] on platforms that
    don't need wrappers (Windows native, or Linux/macOS with native builds).
    """
    if platform != Platform.WSL:
        return []

    step("Installing shell wrappers")
    bin_dir.mkdir(parents=True, exist_ok=True)

    src_dir = _wrappers_source_dir(repo_root)
    resolver_src = src_dir / "_resolve_ai_id.sh"
    if not resolver_src.exists():
        warn(f"Resolver not found at {resolver_src} — wrappers skipped.")
        return []

    resolver_dst = bin_dir / "_resolve_ai_id.sh"
    shutil.copy2(resolver_src, resolver_dst)
    _make_executable(resolver_dst)

    written: list[str] = []
    for name, exe in WRAPPER_BINARIES:
        exe_path = bin_dir / exe
        if not exe_path.exists():
            # Skip wrappers for binaries that weren't installed (e.g. forge skipped)
            continue

        wrapper_path = bin_dir / name
        content = _WRAPPER_TEMPLATE.format(name=name, exe=exe)
        wrapper_path.write_text(content)
        _make_executable(wrapper_path)
        written.append(name)

    ok(f"Installed {len(written)} wrappers + resolver to {bin_dir}")
    if written:
        info(f"  Wrappers: {', '.join(written)}")
    return written
