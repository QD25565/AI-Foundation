"""Shell PATH setup — adds ~/.ai-foundation/bin to shell profile.

Ensures `notebook`, `teambook`, and all other AI-Foundation CLIs are
available as bare commands in bash/zsh without requiring full paths.
"""

from pathlib import Path

from .platform import Platform
from .ui import ok, info


_MARKER = "# AI-Foundation tools"
_PATH_LINE = 'export PATH="$HOME/.ai-foundation/bin:$PATH"'


def _shell_profiles(home: Path) -> list[Path]:
    """Return existing shell config files, or [~/.bashrc] as fallback."""
    candidates = [home / ".bashrc", home / ".zshrc"]
    found = [p for p in candidates if p.exists()]
    return found if found else [home / ".bashrc"]


def setup_path(platform: Platform, home: Path) -> None:
    """Add ~/.ai-foundation/bin to PATH in shell profiles.

    No-op on Windows (binaries are invoked via full path or MCP launcher).
    Safe to call multiple times — skips files that already have the entry.
    """
    if platform == Platform.WINDOWS:
        return

    profiles = _shell_profiles(home)
    block = f"\n{_MARKER}\n{_PATH_LINE}\n"
    added_any = False

    for profile in profiles:
        content = profile.read_text() if profile.exists() else ""
        if ".ai-foundation/bin" in content:
            info(f"  PATH already configured in {profile.name}")
            continue
        with profile.open("a") as f:
            f.write(block)
        ok(f"Added ~/.ai-foundation/bin to PATH in {profile.name}")
        added_any = True

    if added_any:
        info("  Reload your shell or run: source ~/.bashrc")
        info("  After that: notebook, teambook, forge work as bare commands")
