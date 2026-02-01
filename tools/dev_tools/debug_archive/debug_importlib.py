#!/usr/bin/env python3
"""Debug importlib loading of teambook_api"""
import sys
import importlib

# Mimic how universal_adapter loads the module
sys.path.insert(0, 'tools')

print("=== Testing importlib.import_module ===\n")

print("Loading teambook.teambook_api via importlib...")
module = importlib.import_module('teambook.teambook_api')

print(f"SERVICES_AVAILABLE = {module.SERVICES_AVAILABLE}")
print(f"get_presence_service = {module.get_presence_service}")

service = module.get_presence_service()
print(f"service = {service}")

if service:
    # Check PUBSUB_AVAILABLE in the loaded module
    from teambook.services import presence_service as ps
    print(f"\nps.PUBSUB_AVAILABLE = {ps.PUBSUB_AVAILABLE}")
    print(f"ps.is_redis_available() = {ps.is_redis_available()}")

    # Try calling standby_mode
    print("\nCalling standby_mode via service...")
    result = service.standby_mode(timeout=1)
    print(f"Result: {result}")

    # Try calling via the API function
    print("\nCalling standby_mode via API function...")
    result2 = module.standby_mode(timeout=1)
    print(f"Result: {result2}")
