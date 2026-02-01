#!/usr/bin/env python3
"""Debug actual CLI execution path"""
import sys
from pathlib import Path

# Mimic __main__.py setup
parent_dir = Path(__file__).parent / 'tools'
sys.path.insert(0, str(parent_dir.parent / 'tools'))

print("=== Debugging CLI Execution ===\n")

# Import teambook_api like the CLI does
import importlib
module = importlib.import_module('teambook.teambook_api')

print(f"1. SERVICES_AVAILABLE = {module.SERVICES_AVAILABLE}")

# Get the standby_mode function
standby_func = getattr(module, 'standby_mode')
print(f"2. standby_mode function = {standby_func}")

# Check if service is available
service = module.get_presence_service()
print(f"3. service = {service}")

if service:
    # Check the actual module's PUBSUB_AVAILABLE
    import teambook.services.presence_service as ps_mod
    print(f"4. ps_mod.PUBSUB_AVAILABLE = {ps_mod.PUBSUB_AVAILABLE}")
    print(f"5. ps_mod.is_redis_available = {ps_mod.is_redis_available}")

    # Call is_redis_available
    result = ps_mod.is_redis_available()
    print(f"6. ps_mod.is_redis_available() = {result}")

    # Now try calling standby_mode
    print("\n7. Calling standby_mode(timeout=1)...")
    result = standby_func(timeout=1)
    print(f"   Result: {result}")
else:
    print("ERROR: service is None")
