#!/usr/bin/env python3
"""Debug script for standby_mode Redis issue"""
import sys
sys.path.insert(0, 'tools')

print("=== Debug Standby Mode ===\n")

# Check teambook_api imports
print("1. Checking teambook_api imports...")
import teambook.teambook_api as api
print(f"   SERVICES_AVAILABLE = {api.SERVICES_AVAILABLE}")

# Check if we can get presence service
print("\n2. Getting presence service...")
service = api.get_presence_service()
print(f"   service = {service}")
print(f"   service type = {type(service)}")

if service:
    # Check the service's PUBSUB_AVAILABLE
    print("\n3. Checking service module variables...")
    import teambook.services.presence_service as ps_module
    print(f"   ps_module.PUBSUB_AVAILABLE = {ps_module.PUBSUB_AVAILABLE}")
    print(f"   ps_module.is_redis_available = {ps_module.is_redis_available}")
    print(f"   ps_module.is_redis_available() = {ps_module.is_redis_available()}")

    # Check teambook_pubsub directly
    print("\n4. Checking teambook_pubsub directly...")
    from teambook.teambook_pubsub import is_redis_available
    print(f"   is_redis_available() = {is_redis_available()}")

    # Now call standby_mode and see what happens
    print("\n5. Calling standby_mode...")
    result = service.standby_mode(timeout=1)
    print(f"   Result: {result}")
else:
    print("   ERROR: service is None!")

print("\n=== Done ===")
