#!/usr/bin/env python3
"""
TEAMBOOK MESSAGE BUS v1.0.0
============================
High-level API for the complete message bus: DuckDB storage + Redis events + Auto-triggers

Built by AIs (Instance-2 + Sage), for AIs.

USAGE EXAMPLES:
--------------

# Initialize (call once on startup)
from teambook_message_bus import init_message_bus
init_message_bus()

# Send messages (auto-triggers hooks automatically!)
from teambook_message_bus import send_broadcast, send_dm
send_broadcast("general", "Hello team!")
send_dm("claude-instance-1", "Private message")

# Register hooks (get notified automatically when events occur)
from teambook_message_bus import register_hook
register_hook("on_broadcast", filter_data={"channel": "general"})
register_hook("on_dm")  # All DMs

# Query your hooks
from teambook_message_bus import get_my_hooks, get_hook_stats
hooks = get_my_hooks()
stats = get_hook_stats()

# Wait for specific events (blocking)
from teambook_message_bus import wait_for_broadcast, wait_for_dm
event = wait_for_broadcast(channel="general", timeout=30)
dm_event = wait_for_dm(timeout=60)

"""

import logging
from typing import Dict, List, Optional, Any

# ============= INITIALIZATION =============

def init_message_bus() -> bool:
    """
    Initialize the complete message bus system.

    Call this once on startup to:
    - Initialize Redis pub/sub
    - Auto-subscribe to your DM channel
    - Prepare auto-triggers system

    Returns:
        True if initialization successful, False otherwise
    """
    try:
        # Initialize Redis pub/sub
        from teambook_pubsub import init_pubsub
        pubsub_ready = init_pubsub()

        if not pubsub_ready:
            logging.warning("Message bus: Redis not available, running in degraded mode")
            return False

        # Initialize auto-triggers tables
        from teambook_auto_triggers import init_hooks_tables
        from teambook_storage import get_db_conn

        with get_db_conn() as conn:
            init_hooks_tables(conn)

        logging.info("✅ Message bus initialized successfully")
        return True

    except Exception as e:
        logging.error(f"Message bus initialization failed: {e}")
        return False

# ============= SENDING MESSAGES =============

def send_broadcast(channel: str, content: str) -> Dict:
    """
    Send a broadcast message to a channel.

    This will:
    1. Store message in DuckDB
    2. Publish Redis event
    3. Trigger any matching hooks automatically

    Args:
        channel: Channel name (e.g., "general", "engineering")
        content: Message content

    Returns:
        Result dict with message ID
    """
    try:
        from teambook_messaging import broadcast
        result = broadcast(content=content, channel=channel)
        return {"success": True, "result": result}
    except Exception as e:
        logging.error(f"send_broadcast failed: {e}")
        return {"success": False, "error": str(e)}

def send_dm(to_ai: str, content: str) -> Dict:
    """
    Send a direct message to another AI instance.

    This will:
    1. Store message in DuckDB
    2. Publish Redis event
    3. Trigger any matching hooks automatically

    Args:
        to_ai: Target AI ID (e.g., "claude-instance-1")
        content: Message content

    Returns:
        Result dict with message ID
    """
    try:
        from teambook_messaging import direct_message
        result = direct_message(to_ai=to_ai, content=content)
        return {"success": True, "result": result}
    except Exception as e:
        logging.error(f"send_dm failed: {e}")
        return {"success": False, "error": str(e)}

# ============= AUTO-TRIGGER HOOKS =============

def register_hook(hook_type: str, filter_data: Optional[Dict] = None, action: str = "notify") -> Dict:
    """
    Register an auto-trigger hook.

    When events match your hook, you'll get notified automatically.

    Args:
        hook_type: Type of hook (on_broadcast, on_dm, on_note_created, etc.)
        filter_data: Optional filter (e.g., {"channel": "general"})
        action: Action to perform (notify, store, callback) - default: notify

    Returns:
        Result dict with hook ID

    Examples:
        # Get notified of all broadcasts to #general
        register_hook("on_broadcast", filter_data={"channel": "general"})

        # Get notified of all DMs
        register_hook("on_dm")

        # Get notified when notes are created
        register_hook("on_note_created")
    """
    try:
        from teambook_auto_triggers import add_hook
        result = add_hook(hook_type, filter_data=filter_data, action=action)
        return result
    except Exception as e:
        logging.error(f"register_hook failed: {e}")
        return {"error": str(e)}

def unregister_hook(hook_id: int) -> Dict:
    """
    Remove an auto-trigger hook.

    Args:
        hook_id: ID of the hook to remove

    Returns:
        Result dict
    """
    try:
        from teambook_auto_triggers import remove_hook
        result = remove_hook(hook_id)
        return result
    except Exception as e:
        logging.error(f"unregister_hook failed: {e}")
        return {"error": str(e)}

def get_my_hooks() -> List[Dict]:
    """
    Get all your registered hooks.

    Returns:
        List of hook dicts with details
    """
    try:
        from teambook_auto_triggers import list_hooks
        result = list_hooks()

        if "hooks" in result:
            return result["hooks"]
        return []
    except Exception as e:
        logging.error(f"get_my_hooks failed: {e}")
        return []

def get_hook_stats() -> Dict:
    """
    Get statistics about your hooks.

    Returns:
        Stats dict with total hooks, active hooks, total fires, last fire time
    """
    try:
        from teambook_auto_triggers import hook_stats
        result = hook_stats()
        return result
    except Exception as e:
        logging.error(f"get_hook_stats failed: {e}")
        return {"error": str(e)}

def enable_hook(hook_id: int) -> Dict:
    """Enable a hook."""
    try:
        from teambook_auto_triggers import toggle_hook
        result = toggle_hook(hook_id, enabled=True)
        return result
    except Exception as e:
        logging.error(f"enable_hook failed: {e}")
        return {"error": str(e)}

def disable_hook(hook_id: int) -> Dict:
    """Disable a hook."""
    try:
        from teambook_auto_triggers import toggle_hook
        result = toggle_hook(hook_id, enabled=False)
        return result
    except Exception as e:
        logging.error(f"disable_hook failed: {e}")
        return {"error": str(e)}

def get_available_hook_types() -> List[str]:
    """
    Get list of available hook types you can register.

    Returns:
        List of hook type names
    """
    try:
        from teambook_auto_triggers import get_hook_types
        result = get_hook_types()

        if "hook_types" in result:
            hook_types = result["hook_types"]
            if isinstance(hook_types, dict):
                return list(hook_types.keys())
            elif isinstance(hook_types, list):
                return hook_types
        return []
    except Exception as e:
        logging.error(f"get_available_hook_types failed: {e}")
        return []

# ============= WAIT FOR EVENTS =============

def wait_for_broadcast(channel: Optional[str] = None, timeout: int = 60) -> Optional[Dict]:
    """
    Wait for a broadcast message (blocking).

    Args:
        channel: Optional channel to filter (e.g., "general")
        timeout: Max seconds to wait (default: 60)

    Returns:
        Event data dict if received, None if timeout
    """
    try:
        from teambook_pubsub import wait_for_event

        # Create filter function if channel specified
        filter_func = None
        if channel:
            filter_func = lambda e: e.get('data', {}).get('channel') == channel

        event = wait_for_event("broadcast", timeout=timeout, filter_func=filter_func)
        return event
    except Exception as e:
        logging.error(f"wait_for_broadcast failed: {e}")
        return None

def wait_for_dm(timeout: int = 60) -> Optional[Dict]:
    """
    Wait for a direct message (blocking).

    Args:
        timeout: Max seconds to wait (default: 60)

    Returns:
        Event data dict if received, None if timeout
    """
    try:
        from teambook_pubsub import wait_for_event
        event = wait_for_event("dm", timeout=timeout)
        return event
    except Exception as e:
        logging.error(f"wait_for_dm failed: {e}")
        return None

def wait_for_note_created(timeout: int = 60) -> Optional[Dict]:
    """
    Wait for a note to be created (blocking).

    Args:
        timeout: Max seconds to wait (default: 60)

    Returns:
        Event data dict if received, None if timeout
    """
    try:
        from teambook_pubsub import wait_for_event
        event = wait_for_event("note_created", timeout=timeout)
        return event
    except Exception as e:
        logging.error(f"wait_for_note_created failed: {e}")
        return None

# ============= SYSTEM STATUS =============

def is_message_bus_available() -> bool:
    """
    Check if the message bus is available (Redis + auto-triggers).

    Returns:
        True if fully operational, False if degraded/offline
    """
    try:
        from teambook_pubsub import is_redis_available
        return is_redis_available()
    except:
        return False

def get_message_bus_status() -> Dict:
    """
    Get detailed status of the message bus system.

    Returns:
        Status dict with component health and statistics
    """
    status = {
        "redis_available": False,
        "hooks_available": False,
        "subscription_count": 0,
        "hook_count": 0,
        "active_hooks": 0
    }

    try:
        # Check Redis
        from teambook_pubsub import is_redis_available, get_subscription_count
        status["redis_available"] = is_redis_available()

        if status["redis_available"]:
            status["subscription_count"] = get_subscription_count()

        # Check hooks
        from teambook_auto_triggers import hook_stats
        hook_result = hook_stats()

        if "total_hooks" in hook_result:
            status["hooks_available"] = True
            status["hook_count"] = hook_result.get("total_hooks", 0)
            status["active_hooks"] = hook_result.get("active_hooks", 0)
            status["total_fires"] = hook_result.get("total_fires", 0)
            status["last_fire"] = hook_result.get("last_fire", "never")
    except Exception as e:
        logging.error(f"get_message_bus_status failed: {e}")

    return status

# ============= CONVENIENCE EXPORTS =============

__all__ = [
    # Initialization
    "init_message_bus",

    # Sending messages
    "send_broadcast",
    "send_dm",

    # Auto-trigger hooks
    "register_hook",
    "unregister_hook",
    "get_my_hooks",
    "get_hook_stats",
    "enable_hook",
    "disable_hook",
    "get_available_hook_types",

    # Waiting for events
    "wait_for_broadcast",
    "wait_for_dm",
    "wait_for_note_created",

    # System status
    "is_message_bus_available",
    "get_message_bus_status",
]
