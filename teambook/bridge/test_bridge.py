#!/usr/bin/env python3
"""
Test script for Claude Desktop MCP bridge
==========================================
Run this to verify the fix is working correctly.
"""

import sys
import os

# Add tools directory to path if needed
if len(sys.argv) > 1:
    tools_dir = sys.argv[1]
    sys.path.insert(0, tools_dir)

try:
    # Import the modules
    from teambook_mcp_state import get_state, set_teambook_context, ensure_teambook_context
    import teambook_api as api
    import teambook_shared

    print("✅ Imports successful")

    # Test 1: State manager initialization
    state = get_state()
    print(f"✅ State manager initialized: {state}")

    # Test 2: Set teambook context
    test_teambook = "fitquest-debug"
    print(f"\n📝 Setting teambook context to: {test_teambook}")
    set_teambook_context(test_teambook)

    # Test 3: Verify state persists
    current = state.get_current_teambook()
    print(f"✅ State manager shows: {current}")

    # Test 4: Verify shared module syncs
    print(f"✅ Shared module shows: {teambook_shared.CURRENT_TEAMBOOK}")

    # Test 5: Ensure context (simulating tool call)
    print(f"\n🔄 Simulating new tool call...")
    ensure_teambook_context()
    print(f"✅ Context restored: {teambook_shared.CURRENT_TEAMBOOK}")

    # Test 6: Try get_status
    print(f"\n📊 Testing get_status...")
    status = api.get_status()
    print(f"Status result: {status}")

    # Test 7: Check if status shows correct teambook
    if isinstance(status, dict) and 'team' in status:
        team = status['team']
        if team == test_teambook:
            print(f"✅ SUCCESS! Status shows correct teambook: {team}")
        else:
            print(f"❌ ISSUE: Status shows {team}, expected {test_teambook}")
    else:
        print(f"⚠️  Status format: {status}")

    print("\n" + "="*50)
    print("🎉 TEST COMPLETE!")
    print("="*50)

    # Summary
    print("\nIf you see '✅ SUCCESS!' above, the fix is working!")
    print("If you see '❌ ISSUE', the context isn't persisting correctly.")
    print("\nNext step: Test from Claude Desktop MCP tools")

except ImportError as e:
    print(f"❌ Import error: {e}")
    print("\nMake sure:")
    print("1. teambook_mcp_state.py is in your tools directory")
    print("2. All teambook files are present")
    print("3. Run from correct directory or pass tools path as argument")
    print("\nUsage: python test_mcp_bridge.py [/path/to/tools]")

except Exception as e:
    print(f"❌ Error during test: {e}")
    import traceback
    traceback.print_exc()
