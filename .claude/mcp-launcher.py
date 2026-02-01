#!/usr/bin/env python3
"""
Cross-platform MCP server launcher for AI-Foundation.
Finds the correct MCP binary regardless of platform (Windows/WSL/Linux)
and execs it with proper env var forwarding.

Since MCP servers communicate via stdio (JSON-RPC), this launcher uses
os.execvpe() to replace itself with the binary — no stdin/stdout buffering.

Usage in .mcp.json:
  "command": "python3",
  "args": [".claude/mcp-launcher.py", "ai-foundation-mcp-ENGRAM-v51"]
"""
import os
import sys

# Import platform_utils from hooks directory
hooks_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'hooks')
sys.path.insert(0, hooks_dir)

from platform_utils import (
    find_ai_foundation_bin,
    get_binary,
    is_wsl,
    prepare_env_for_exe,
)


def convert_env_paths_for_wsl(env):
    """On WSL, env vars with Windows paths (C:/...) need to stay as-is
    because the target binary is a Windows .exe that expects Windows paths.

    However, if we're on native Linux, convert Windows paths to Linux paths.
    On WSL, no conversion needed since the .exe is a Windows process.
    """
    if is_wsl() or sys.platform == 'win32':
        return  # .exe expects Windows paths, leave them alone

    # Native Linux: convert any Windows-style paths
    path_vars = ['GOOGLE_APPLICATION_CREDENTIALS', 'MCP_INSTANCE_ROOT']
    for var in path_vars:
        val = env.get(var, '')
        if val and len(val) > 2 and val[1] == ':':
            drive = val[0].lower()
            rest = val[2:].replace('\\', '/')
            env[var] = f'/mnt/{drive}{rest}'


def main():
    if len(sys.argv) < 2:
        print("Usage: mcp-launcher.py <binary-name> [args...]", file=sys.stderr)
        sys.exit(1)

    binary_name = sys.argv[1]
    extra_args = sys.argv[2:]

    # Find the binary
    binary = get_binary(binary_name)
    if binary is None:
        bin_dir = find_ai_foundation_bin()
        print(
            f"ERROR: Could not find binary '{binary_name}'. "
            f"Bin dir: {bin_dir}",
            file=sys.stderr,
        )
        sys.exit(1)

    binary_path = str(binary)

    # Prepare environment
    env = os.environ.copy()

    # Forward env vars to Windows .exe on WSL
    prepare_env_for_exe(
        env,
        extra_keys=['GOOGLE_APPLICATION_CREDENTIALS', 'TEAMENGRAM_V2'],
    )

    # Convert paths if needed (native Linux only)
    convert_env_paths_for_wsl(env)

    # Replace this process with the MCP binary.
    # os.execvpe replaces the current process entirely — stdin/stdout
    # pass through to the new process without any Python buffering.
    try:
        os.execvpe(binary_path, [binary_path] + extra_args, env)
    except OSError as e:
        print(f"ERROR: Failed to exec '{binary_path}': {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
