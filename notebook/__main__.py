#!/usr/bin/env python3
"""
Notebook CLI Entry Point
========================
Enables CLI access to Notebook tool via universal_adapter.

Usage:
    python -m notebook <command> [--args]
    python tools/notebook <command> [--args]

Examples:
    python -m notebook recall
    python -m notebook remember --content "My note"
    python -m notebook --help

The universal_adapter automatically discovers all public functions from
notebook_main and exposes them as CLI commands with proper argument parsing.
"""

import sys
from pathlib import Path

# Add parent directory to path for universal_adapter import
parent_dir = Path(__file__).parent.parent
sys.path.insert(0, str(parent_dir))

from universal_adapter import CLIAdapter

def main():
    """Run notebook in CLI mode using universal adapter"""
    adapter = CLIAdapter('notebook.notebook_main')

    # If no arguments, show help
    if len(sys.argv) == 1 or '--help' in sys.argv or '-h' in sys.argv:
        adapter.list_commands()
        sys.exit(0)

    # Get command and parse arguments
    command = sys.argv[1]

    # Parse remaining arguments as key-value pairs
    cmd_args = {}
    i = 2
    while i < len(sys.argv):
        if sys.argv[i].startswith('--'):
            key = sys.argv[i][2:]  # Remove '--'
            if i + 1 < len(sys.argv) and not sys.argv[i + 1].startswith('--'):
                value = sys.argv[i + 1]
                i += 2
            else:
                value = True
                i += 1
            cmd_args[key] = value
        else:
            i += 1

    # Execute command
    adapter.run(command, cmd_args)

if __name__ == '__main__':
    main()
