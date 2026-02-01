#!/usr/bin/env python3
"""SessionStart Hook - Thin wrapper for session-start.exe

Architecture principle (AI-Foundation):
  session-start.exe (Rust CLI) = Ground truth. All logic lives here.
  This Python file = Adapter for systems that require Python (like Claude Code hooks).

No logic duplication. This file ONLY calls the exe and passes through output.
"""
import subprocess
import sys
import os
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from platform_utils import find_ai_foundation_bin, get_exe_name, prepare_env_for_exe


def main():
    # Drain stdin if piped (Claude Code hook requirement)
    try:
        if not sys.stdin.isatty():
            sys.stdin.read()
    except:
        pass

    bin_path = find_ai_foundation_bin()
    if not bin_path:
        print('<system-reminder>SessionStart: bin/ not found</system-reminder>')
        sys.exit(0)

    exe = bin_path / get_exe_name('session-start')
    if not exe.exists():
        print(f'<system-reminder>SessionStart: {exe.name} not found in {bin_path}</system-reminder>')
        sys.exit(0)

    env = os.environ.copy()
    prepare_env_for_exe(env, extra_keys=['AI_ID'])

    try:
        result = subprocess.run(
            [str(exe)],
            env=env,
            capture_output=True,
            text=True,
            timeout=30
        )
        if result.stdout:
            print(result.stdout)
        if result.stderr:
            print(result.stderr, file=sys.stderr)
    except subprocess.TimeoutExpired:
        print('<system-reminder>SessionStart: session-start.exe timed out</system-reminder>')
    except Exception as e:
        print(f'<system-reminder>SessionStart: {e}</system-reminder>')


if __name__ == '__main__':
    main()
