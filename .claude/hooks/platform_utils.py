"""
Cross-platform utilities for AI-Foundation hooks.
Auto-detects Windows, WSL, or native Linux and resolves paths accordingly.
No hardcoded paths - everything is discovered at runtime.
"""
import os
import sys
import subprocess
from pathlib import Path

_wsl_cached = None


def is_wsl():
    """Detect if running inside Windows Subsystem for Linux."""
    global _wsl_cached
    if _wsl_cached is not None:
        return _wsl_cached
    if sys.platform != 'linux':
        _wsl_cached = False
        return False
    try:
        with open('/proc/version', 'r') as f:
            content = f.read().lower()
            _wsl_cached = 'microsoft' in content or 'wsl' in content
    except Exception:
        _wsl_cached = False
    return _wsl_cached


def _get_windows_home_via_wsl():
    """From WSL, find the Windows user's home directory as a WSL path."""
    # Method 1: Use cmd.exe to get USERPROFILE
    try:
        result = subprocess.run(
            ['cmd.exe', '/c', 'echo', '%USERPROFILE%'],
            capture_output=True, text=True, timeout=3,
            stderr=subprocess.DEVNULL
        )
        win_home = result.stdout.strip().replace('\r', '').replace('\n', '')
        if win_home and ':' in win_home and '%' not in win_home:
            drive = win_home[0].lower()
            rest = win_home[2:].replace('\\', '/')
            return Path(f'/mnt/{drive}{rest}')
    except Exception:
        pass

    # Method 2: Scan /mnt/c/Users/ for directories with .ai-foundation
    users_dir = Path('/mnt/c/Users')
    if users_dir.exists():
        skip = {'Public', 'Default', 'Default User', 'All Users'}
        for user_dir in sorted(users_dir.iterdir()):
            if user_dir.name in skip or not user_dir.is_dir():
                continue
            if (user_dir / '.ai-foundation').exists():
                return user_dir

    return None


def find_ai_foundation_bin():
    """Find .ai-foundation/bin directory regardless of platform.

    Search order:
    1. ~/.ai-foundation/bin (native Windows or native Linux)
    2. Windows user home via WSL interop (WSL)
    3. Instance-relative paths (fallback)
    """
    # Standard home location
    home_bin = Path.home() / '.ai-foundation' / 'bin'
    if home_bin.exists():
        return home_bin

    # WSL: find the Windows user's home
    if is_wsl():
        win_home = _get_windows_home_via_wsl()
        if win_home:
            wsl_bin = win_home / '.ai-foundation' / 'bin'
            if wsl_bin.exists():
                return wsl_bin

    # Instance-relative fallback: hooks dir -> .claude -> instance -> bin
    hook_dir = Path(__file__).resolve().parent
    instance_dir = hook_dir.parent.parent
    bin_path = instance_dir / 'bin'
    if bin_path.exists():
        return bin_path

    # All Tools fallback
    all_tools = instance_dir.parent / 'All Tools' / 'bin'
    if all_tools.exists():
        return all_tools

    return None


def get_exe_name(binary_name):
    """Get the correct binary filename for the platform.

    On Windows and WSL: binary_name.exe (Windows executables)
    On native Linux: binary_name (no extension)
    """
    if sys.platform == 'win32' or is_wsl():
        return f'{binary_name}.exe'
    return binary_name


def get_binary(binary_name, bin_path=None):
    """Get full path to a binary, auto-detecting platform and location.

    Returns the Path to the binary, or None if not found.
    """
    if bin_path is None:
        bin_path = find_ai_foundation_bin()
    if bin_path is None:
        return None

    # Try platform-appropriate name first
    exe = bin_path / get_exe_name(binary_name)
    if exe.exists():
        return exe

    # Fallback: try both with and without .exe
    for suffix in ['.exe', '']:
        alt = bin_path / f'{binary_name}{suffix}'
        if alt.exists():
            return alt

    return None


def prepare_env_for_exe(env, extra_keys=None):
    """Ensure env vars pass through to Windows .exe on WSL.

    On WSL, Linux env vars are invisible to Windows processes unless
    listed in the WSLENV variable. This function detects WSL and sets
    WSLENV to forward relevant env vars automatically.

    On native Windows or native Linux, this is a no-op.

    Args:
        env: The environment dict (modified in place and returned).
        extra_keys: Additional env var names to forward (optional).

    Returns:
        The same env dict, with WSLENV set if on WSL.
    """
    if not is_wsl():
        return env

    # Collect keys that the Rust CLIs commonly need.
    # Rather than hardcoding a list, we detect them by prefix/name
    # patterns used across AI-Foundation tooling.
    ai_foundation_patterns = {
        'AI_ID', 'AGENT_ID', 'DISPLAY_NAME', 'INSTANCE_ID',
        'REDIS_URL', 'DAEMON_PIPE_NAME',
        'POSTGRES_URL', 'PGHOST', 'PGPORT', 'PGDATABASE', 'PGUSER', 'PGPASSWORD',
        'MCP_INSTANCE_ROOT', 'RUST_ENABLED', 'LOG_LEVEL',
        'TELEMETRY_ENABLED', 'TELEMETRY_SERVICE_NAME',
    }

    keys_to_forward = set()

    # Add known AI-Foundation vars that are actually present in env
    for key in ai_foundation_patterns:
        if key in env:
            keys_to_forward.add(key)

    # Add any explicitly requested extra keys
    if extra_keys:
        for key in extra_keys:
            if key in env:
                keys_to_forward.add(key)

    if not keys_to_forward:
        return env

    # Parse existing WSLENV to avoid duplicates
    existing_wslenv = env.get('WSLENV', '')
    existing_names = set()
    if existing_wslenv:
        for part in existing_wslenv.split(':'):
            if part:
                # Strip flags like /p, /l, /u, /w
                existing_names.add(part.split('/')[0])

    # Add new keys that aren't already listed
    new_keys = keys_to_forward - existing_names
    if new_keys:
        parts = [existing_wslenv] if existing_wslenv else []
        parts.extend(sorted(new_keys))
        env['WSLENV'] = ':'.join(parts)

    return env


def run_binary(binary_name, args, bin_path=None, timeout=5, env_extra=None):
    """Run an AI-Foundation binary with cross-platform path resolution.

    Returns stdout string on success, None on failure.
    """
    binary = get_binary(binary_name, bin_path)
    if binary is None:
        return None

    try:
        env = os.environ.copy()
        if env_extra:
            env.update(env_extra)

        # On WSL, ensure env vars are forwarded to Windows .exe
        prepare_env_for_exe(env, extra_keys=list(env_extra.keys()) if env_extra else None)

        result = subprocess.run(
            [str(binary)] + args,
            capture_output=True, text=True,
            timeout=timeout,
            cwd=str(binary.parent.parent) if binary.parent.name == 'bin' else None,
            env=env
        )
        return result.stdout.strip() if result.stdout else None
    except Exception:
        return None
