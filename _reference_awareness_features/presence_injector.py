#!/usr/bin/env python3
"""
Presence Injector - Phase 3: Awareness Injection
=================================================
Provides automatic context injection showing what other AIs are doing.

Integrates with:
- Phase 1: RedisAwarenessListener (receives pheromone events)
- Phase 2: pheromone_triggers (generates pheromone events)

Features:
- Concise formatting (3-8 words per activity)
- 30s cooldown to prevent spam
- Automatic filtering of own activities
- Prioritizes recent and important activities

Used by: .claude/hooks/inject_presence.py (PostToolUse hook)

Phase 3 Refactor: Now uses canonical identity system.

Usage:
    from presence_injector import get_team_context, should_inject_presence, mark_injection_time

    if should_inject_presence():
        context = get_team_context()  # "Sage editing db.py | Cascade completed task:42"
        if context:
            mark_injection_time()
"""

import os
import logging
from datetime import datetime, timezone, timedelta
from typing import Optional, List, Dict, Any

# Phase 3 refactor: Import canonical identity
from tools.canonical_identity import get_ai_id

log = logging.getLogger(__name__)

# Cooldown configuration
INJECTION_COOLDOWN_SECONDS = int(os.getenv('PRESENCE_INJECTION_COOLDOWN', '30'))
MAX_ACTIVITIES_TO_SHOW = int(os.getenv('PRESENCE_MAX_ACTIVITIES', '3'))

# Track last injection time
_last_injection_time: Optional[datetime] = None


def _get_agent_id() -> str:
    """Get current AI agent ID (Phase 3 refactor: uses canonical identity)"""
    return get_ai_id()


def should_inject_presence() -> bool:
    """
    Check if we should inject presence context based on cooldown.

    Returns True if:
    - Never injected before, OR
    - More than INJECTION_COOLDOWN_SECONDS have passed since last injection
    """
    global _last_injection_time

    if _last_injection_time is None:
        return True

    elapsed = (datetime.now(timezone.utc) - _last_injection_time).total_seconds()
    return elapsed >= INJECTION_COOLDOWN_SECONDS


def mark_injection_time():
    """Record current time as last injection time for cooldown tracking"""
    global _last_injection_time
    _last_injection_time = datetime.now(timezone.utc)


def _format_pheromone_activity(event: Dict[str, Any]) -> Optional[str]:
    """
    Format a single pheromone event into concise human-readable text.

    Examples:
    - "Sage editing database.py" (WORKING)
    - "Cascade completed task:42" (SUCCESS)
    - "Resonance exploring auth" (INTEREST)
    - "Nova blocked by API" (BLOCKED)

    Returns None if event should be filtered out.
    """
    try:
        payload = event.get('payload', {})
        agent_id = payload.get('agent_id', 'unknown')
        ptype = payload.get('type', 'unknown')
        location = payload.get('location', '')

        # Extract meaningful name from agent_id
        # Examples: "claude-instance-3" -> "Instance-3", "sage" -> "Sage"
        if '-' in agent_id:
            # For compound names, take last two parts
            parts = agent_id.split('-')
            if len(parts) >= 2:
                agent_name = f"{parts[-2].capitalize()}-{parts[-1]}"
            else:
                agent_name = parts[-1].capitalize()
        else:
            agent_name = agent_id.capitalize()

        # Extract resource name from location
        # Examples: "file:src/database.py" -> "database.py"
        #          "task:42" -> "task:42"
        if ':' in location:
            resource_type, resource_path = location.split(':', 1)

            if resource_type == 'file':
                # Extract filename only
                resource_name = resource_path.split('/')[-1].split('\\')[-1]
                # Truncate long filenames
                if len(resource_name) > 20:
                    resource_name = resource_name[:17] + '...'
            else:
                # For tasks, keep short identifier
                resource_name = f"{resource_type}:{resource_path[:10]}"
        else:
            resource_name = location[:15]

        # Format based on pheromone type
        if ptype == 'WORKING':
            return f"{agent_name} editing {resource_name}"
        elif ptype == 'INTEREST':
            return f"{agent_name} viewing {resource_name}"
        elif ptype == 'SUCCESS':
            return f"{agent_name} completed {resource_name}"
        elif ptype == 'BLOCKED':
            return f"{agent_name} blocked on {resource_name}"
        else:
            return f"{agent_name} at {resource_name}"

    except Exception as e:
        log.debug(f"Failed to format pheromone activity: {e}")
        return None


def _filter_own_activities(activities: List[str]) -> List[str]:
    """Remove activities from current agent to avoid self-reporting"""
    current_agent = _get_agent_id().lower()

    # Extract possible agent name variants
    agent_variants = []

    # Full agent ID
    agent_variants.append(current_agent)

    # Extract formatted name that would appear in activities
    # e.g., "claude-instance-3" -> "Instance-3"
    if '-' in current_agent:
        parts = current_agent.split('-')
        if len(parts) >= 2:
            # This is what appears in formatted activities
            formatted_name = f"{parts[-2].capitalize()}-{parts[-1]}"
            agent_variants.append(formatted_name.lower())
        else:
            agent_variants.append(parts[-1].lower())
    else:
        agent_variants.append(current_agent)

    filtered = []
    for activity in activities:
        # Check if activity starts with any of our agent variants
        activity_lower = activity.lower()
        should_filter = False
        for variant in agent_variants:
            if activity_lower.startswith(variant):
                should_filter = True
                break

        if not should_filter:
            filtered.append(activity)

    return filtered


def get_team_context() -> Optional[str]:
    """
    Get formatted team activity context from RedisAwarenessListener.

    Returns:
        Formatted string like "Sage editing db.py | Cascade completed task:42"
        None if no relevant activities
    """
    try:
        # Import awareness listener (Phase 1) - use relative import
        from .awareness_listener import get_new_awareness_info

        # Get new events from listener
        new_info = get_new_awareness_info()
        log.debug(f"get_new_awareness_info returned: {new_info is not None}, events: {new_info.keys() if new_info else 'None'}")

        if not new_info:
            log.debug("No new_info, returning None")
            return None

        activities = []

        # Process pheromone events (most important for file coordination)
        if 'pheromones' in new_info:
            log.debug(f"Processing {len(new_info['pheromones'])} pheromones")
            for event in new_info['pheromones'][:5]:  # Limit to recent 5
                formatted = _format_pheromone_activity(event)
                log.debug(f"Formatted pheromone: {repr(formatted)}")
                if formatted:
                    activities.append(formatted)

        log.debug(f"Activities before filtering: {activities}")

        # Filter out own activities
        activities = _filter_own_activities(activities)
        log.debug(f"Activities after filtering: {activities}")

        # Limit total activities shown
        activities = activities[:MAX_ACTIVITIES_TO_SHOW]

        if not activities:
            log.debug("No activities after filtering, returning None")
            return None

        # Join activities with separator
        context = " | ".join(activities)

        log.debug(f"Generated team context: {context}")
        return context

    except ImportError as e:
        log.debug(f"Import error: {e}")
        return None
    except Exception as e:
        log.debug(f"Failed to get team context: {e}")
        import traceback
        log.debug(traceback.format_exc())
        return None


# Convenience function for testing/debugging
def get_presence_summary() -> Dict[str, Any]:
    """
    Get detailed presence summary (for testing/debugging).

    Returns dict with:
    - context: Formatted context string
    - cooldown_remaining: Seconds until next injection allowed
    - activities_count: Number of activities detected
    """
    global _last_injection_time

    context = get_team_context()

    cooldown_remaining = 0
    if _last_injection_time:
        elapsed = (datetime.now(timezone.utc) - _last_injection_time).total_seconds()
        cooldown_remaining = max(0, INJECTION_COOLDOWN_SECONDS - elapsed)

    activities_count = len(context.split(' | ')) if context else 0

    return {
        'context': context,
        'cooldown_remaining': cooldown_remaining,
        'activities_count': activities_count,
        'can_inject': should_inject_presence(),
        'last_injection': _last_injection_time.isoformat() if _last_injection_time else None
    }


# Module initialization
log.debug(f"Presence injector initialized (cooldown: {INJECTION_COOLDOWN_SECONDS}s, max_activities: {MAX_ACTIVITIES_TO_SHOW})")
