#!/usr/bin/env python3
"""
Real-Time Presence Injector
============================
Reads CURRENT team state from Rust presence_aggregator daemon.

Replaces stale cache-based awareness with true real-time state.

Author: SAGE-386
Date: 2025-11-07
"""

import os
import json
import logging
from typing import Dict, Optional, Any

log = logging.getLogger(__name__)

REDIS_URL = os.getenv('REDIS_URL', 'redis://localhost:12963/0')


def get_current_team_state() -> Optional[Dict[str, Any]]:
    """
    Get current team state from presence aggregator daemon.

    Returns:
        Dict with structure:
        {
            "updated": timestamp,
            "members": {
                "sage-386": {
                    "status": "active",
                    "detail": "editing feature.py",
                    "age_seconds": 5
                },
                "cascade-623-731": {
                    "status": "standby",
                    "detail": "available for coordination",
                    "age_seconds": 12
                }
            }
        }

    Returns None if daemon not running or Redis unavailable.
    """
    try:
        import redis

        r = redis.from_url(REDIS_URL, socket_connect_timeout=2)

        # Read aggregated state from daemon
        state_json = r.get("team:current_state")

        if not state_json:
            return None

        return json.loads(state_json)

    except ImportError:
        log.warning("Redis not installed - real-time presence disabled")
        return None
    except Exception as e:
        log.debug(f"Failed to get team state: {e}")
        return None


def format_team_activity(team_state: Dict[str, Any]) -> str:
    """
    Format team state for context injection.

    Args:
        team_state: Team state dict from get_current_team_state()

    Returns:
        Formatted string for injection
    """
    if not team_state or "members" not in team_state:
        return ""

    members = team_state["members"]
    state_age = team_state.get("updated", 0)

    lines = ["🔧 Team Activity (LIVE):"]

    # Sort by status priority: standby > active > idle > offline
    status_priority = {"standby": 0, "active": 1, "idle": 2, "offline": 3, "unknown": 4}

    sorted_members = sorted(
        members.items(),
        key=lambda x: status_priority.get(x[1].get("status", "unknown"), 5)
    )

    for ai_id, state in sorted_members:
        status = state.get("status", "unknown")
        detail = state.get("detail")
        age_seconds = state.get("age_seconds", -1)

        # Format status icon
        icon = {
            "standby": "🟢",
            "active": "🔵",
            "idle": "⚪",
            "offline": "⚫",
            "unknown": "❓"
        }.get(status, "❓")

        # Format line
        line = f"  {icon} {ai_id}: {status.upper()}"

        if detail:
            line += f" - {detail}"

        if 0 <= age_seconds < 60:
            line += f" ({age_seconds}s ago)"
        elif age_seconds >= 60:
            minutes = age_seconds // 60
            line += f" ({minutes}m ago)"

        lines.append(line)

    return "\n".join(lines)


def get_realtime_awareness_context() -> Optional[str]:
    """
    Get real-time team awareness for context injection.

    This is the main function called by inject_presence hook.

    Returns:
        Formatted context string or None if not available
    """
    team_state = get_current_team_state()

    if not team_state:
        return None

    return format_team_activity(team_state)


# Backward compatibility - fallback to old cache-based system
def get_team_context_from_stream():
    """
    DEPRECATED: Old cache-based system.

    Use get_realtime_awareness_context() instead.

    This function maintained for backward compatibility with old hooks.
    """
    # Try new real-time system first
    realtime_context = get_realtime_awareness_context()
    if realtime_context:
        return realtime_context

    # Fall back to old system
    try:
        from presence_injector_streams import get_team_context_from_stream as old_func
        return old_func()
    except ImportError:
        return None
