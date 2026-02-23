"""Set up Forge CLI configuration at ~/.forge/."""

import shutil
from pathlib import Path

from .ui import ok, info, step, warn


def configure(repo_root: Path, home: Path, ai_id: str) -> None:
    """
    Set up Forge at ~/.forge/:
      - ~/.forge/config.toml (from template, with AI_ID substituted)
      - ~/.forge/models/ (empty directory for GGUF files)
    """
    step("Configuring Forge")

    forge_dir = home / ".forge"
    models_dir = forge_dir / "models"
    config_path = forge_dir / "config.toml"

    forge_dir.mkdir(exist_ok=True)
    models_dir.mkdir(exist_ok=True)
    info(f"  Forge directory: {forge_dir}")

    template = repo_root / "config" / "forge" / "config.toml.template"
    if not template.exists():
        warn("Forge config template not found — skipping config generation")
        return

    if config_path.exists():
        # Preserve existing config — don't overwrite API keys
        ok("Forge already configured (preserving existing ~/.forge/config.toml)")
        info("  To reset: delete ~/.forge/config.toml and re-run the installer")
        return

    content = template.read_text()
    content = content.replace("YOUR_AI_ID", ai_id)
    config_path.write_text(content)

    ok("Forge configured")
    info(f"  Config: {config_path}")
    info("  Next: add your API keys to ~/.forge/config.toml")
    info("  Models: place .gguf files in ~/.forge/models/ for local inference (forge-local.exe)")
