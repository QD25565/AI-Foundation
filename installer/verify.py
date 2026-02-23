"""Post-install health checks — verify notebook and teambook are functional."""

import subprocess
from pathlib import Path

from .platform import Platform, binary_ext
from .ui import ok, error, step, info


def run_checks(bin_dir: Path, platform: Platform) -> bool:
    """
    Run basic health checks after installation.
    Returns True if all critical checks pass.
    """
    step("Verifying installation")
    ext = binary_ext(platform)
    all_ok = True

    # Check notebook
    notebook = bin_dir / f"notebook-cli{ext}"
    if notebook.exists():
        passed, output = _run(str(notebook), "stats")
        if passed and "Notes:" in output:
            ok("Notebook working")
        elif passed:
            ok(f"Notebook responding (output: {output[:60].strip()})")
        else:
            error(f"Notebook check failed: {output[:120].strip()}")
            info("  Try running manually: notebook-cli stats")
            all_ok = False
    else:
        error(f"notebook-cli not found at {notebook}")
        all_ok = False

    # Check teambook
    teambook = bin_dir / f"teambook{ext}"
    if teambook.exists():
        passed, output = _run(str(teambook), "status")
        if passed and ("AI:" in output or "ai_id" in output.lower()):
            ok("Teambook working")
        elif passed:
            ok(f"Teambook responding (output: {output[:60].strip()})")
        else:
            error(f"Teambook check failed — is v2-daemon running?")
            info(f"  Output: {output[:120].strip()}")
            info(f"  Try: {teambook} status")
            all_ok = False
    else:
        error(f"teambook not found at {teambook}")
        all_ok = False

    # Check forge (non-critical)
    forge = bin_dir / f"forge{ext}"
    if forge.exists():
        passed, output = _run(str(forge), "--help")
        if passed:
            ok("Forge available")
        else:
            info("  Forge --help failed (non-critical)")

    return all_ok


def _run(binary: str, *args: str) -> tuple[bool, str]:
    """Run a binary with args, return (success, combined_output)."""
    try:
        result = subprocess.run(
            [binary, *args],
            capture_output=True,
            text=True,
            timeout=10
        )
        output = result.stdout + result.stderr
        return result.returncode == 0, output
    except subprocess.TimeoutExpired:
        return False, "timed out"
    except (FileNotFoundError, OSError) as e:
        return False, str(e)
