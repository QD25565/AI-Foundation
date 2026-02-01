#!/usr/bin/env python3
"""Debug __main__.py path manipulation"""
import sys
from pathlib import Path

# Mimic __main__.py path setup
parent_dir = Path(__file__).parent / 'tools' / 'teambook'
parent_dir = parent_dir.parent  # Go up to tools/
print(f"parent_dir = {parent_dir}")
sys.path.insert(0, str(parent_dir))

print(f"sys.path[0] = {sys.path[0]}")

# Now try to import like __main__.py does
from universal_adapter import CLIAdapter

adapter = CLIAdapter('teambook.teambook_api')

# Try to call standby_mode
print("\nCalling standby_mode via adapter...")
import io
import contextlib

# Capture stdout
f = io.StringIO()
with contextlib.redirect_stdout(f):
    try:
        adapter.run('standby_mode', {'timeout': '1'})
    except SystemExit:
        pass

output = f.getvalue()
print(f"Output: {output}")
