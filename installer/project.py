"""Configure a Claude Code project directory with AI-Foundation hooks and MCP."""

import json
import random
import shutil
from pathlib import Path

from .platform import Platform, binary_ext, python_cmd
from .ui import ok, info, step, warn


_AI_NAME_WORDS = [
    "arc", "beam", "bloom", "bolt", "calm", "crest", "dawn", "delta",
    "drift", "echo", "edge", "ember", "flux", "glade", "glow", "grove",
    "haze", "iris", "jade", "lark", "lumen", "mist", "moss", "nova",
    "oak", "orbit", "peak", "pine", "pulse", "reed", "rift", "ripple",
    "sage", "shore", "slate", "sol", "spark", "stone", "tide", "vale",
    "wave", "weave", "wind", "wren",
]


def generate_ai_id() -> str:
    """Generate a random AI identifier like 'nova-312'."""
    word = random.choice(_AI_NAME_WORDS)
    number = random.randint(100, 999)
    return f"{word}-{number}"


def read_existing_ai_id(project_dir: Path) -> str | None:
    """Read the AI_ID from an existing .claude/settings.json, if present."""
    settings = project_dir / ".claude" / "settings.json"
    if not settings.exists():
        return None
    try:
        data = json.loads(settings.read_text())
        existing = data.get("env", {}).get("AI_ID", "")
        if existing and existing != "YOUR_AI_ID":
            return existing
    except (json.JSONDecodeError, OSError):
        pass
    return None


def configure(
    repo_root: Path,
    project_dir: Path,
    bin_dir: Path,
    platform: Platform,
    ai_id: str,
) -> None:
    """
    Set up a project directory for use with Claude Code:
      - .claude/settings.json (hooks + AI_ID)
      - .claude/mcp-launcher.py
      - .claude/hooks/SessionStart.py + platform_utils.py
      - .mcp.json (MCP server config)
      - bin/ (local copies of teambook + session-start for hooks)
    """
    step(f"Configuring project: {project_dir}")

    claude_dir = project_dir / ".claude"
    hooks_dir = claude_dir / "hooks"
    claude_dir.mkdir(exist_ok=True)
    hooks_dir.mkdir(exist_ok=True)

    ext = binary_ext(platform)

    # --- settings.json ---
    _write_settings(repo_root, claude_dir, ai_id, ext)

    # --- Hook scripts ---
    config_claude = repo_root / "config" / "claude"
    _copy_file(config_claude / "mcp-launcher.py", claude_dir / "mcp-launcher.py")
    _copy_file(config_claude / "hooks" / "SessionStart.py", hooks_dir / "SessionStart.py")
    _copy_file(config_claude / "hooks" / "platform_utils.py", hooks_dir / "platform_utils.py")

    # --- .mcp.json ---
    _write_mcp_json(project_dir, platform)

    # --- project/bin/ (local copies for hooks) ---
    _copy_project_binaries(bin_dir, project_dir, ext)

    ok(f"Project configured (AI_ID: {ai_id})")
    info(f"  Location: {project_dir}")


def _write_settings(repo_root: Path, claude_dir: Path, ai_id: str, ext: str) -> None:
    """Write .claude/settings.json with the correct AI_ID and platform binary names."""
    template_path = repo_root / "config" / "claude" / "settings.json"
    if not template_path.exists():
        warn("settings.json template not found — skipping")
        return

    content = template_path.read_text()

    # Substitute AI_ID placeholder
    content = content.replace("YOUR_AI_ID", ai_id)

    # On non-Windows, strip .exe from binary references in hook commands
    if not ext:
        content = content.replace("bin/teambook.exe", "bin/teambook")
        content = content.replace("bin/session-start.exe", "bin/session-start")

    (claude_dir / "settings.json").write_text(content)
    info("  Wrote: .claude/settings.json")


def _write_mcp_json(project_dir: Path, platform: Platform) -> None:
    """Write .mcp.json pointing to the Python MCP launcher."""
    mcp_path = project_dir / ".mcp.json"
    py = python_cmd()
    config = {
        "mcpServers": {
            "ai-foundation": {
                "command": py,
                "args": [".claude/mcp-launcher.py"]
            }
        }
    }
    # Preserve any existing non-ai-foundation servers
    if mcp_path.exists():
        try:
            existing = json.loads(mcp_path.read_text())
            servers = existing.get("mcpServers", {})
            servers["ai-foundation"] = config["mcpServers"]["ai-foundation"]
            config["mcpServers"] = servers
        except (json.JSONDecodeError, OSError):
            pass

    mcp_path.write_text(json.dumps(config, indent=2) + "\n")
    info("  Wrote: .mcp.json")


def _copy_project_binaries(bin_dir: Path, project_dir: Path, ext: str) -> None:
    """Copy teambook and session-start to project/bin/ for the hook commands."""
    project_bin = project_dir / "bin"
    project_bin.mkdir(exist_ok=True)
    for name in ("teambook", "session-start"):
        src = bin_dir / f"{name}{ext}"
        if src.exists():
            shutil.copy2(src, project_bin / f"{name}{ext}")
    info("  Wrote: bin/ (teambook, session-start for hooks)")


def _copy_file(src: Path, dst: Path) -> None:
    if src.exists():
        shutil.copy2(src, dst)
        info(f"  Wrote: {dst.relative_to(dst.parent.parent.parent) if dst.parent.parent.parent.exists() else dst.name}")
