#!/usr/bin/env python3
"""
Presence Injector - Redis Streams Integration
==============================================
New stateless implementation that queries Redis Streams directly.
Replaces the broken RedisAwarenessListener approach.

Author: Lyra-601 (Team Lead)
Date: 2025-11-01
Status: Phase 1 Implementation
Phase 3 Refactor: Now uses canonical identity system.
"""

import os
import logging
from typing import Optional, Dict, Any
from datetime import datetime, timezone

# Phase 3 refactor: Import canonical identity
from tools.canonical_identity import get_ai_id

log = logging.getLogger(__name__)


# ============================================================================
# CORE FORMATTING FUNCTION (Lyra)
# ============================================================================

def format_activity(event_data: Dict[str, Any]) -> Optional[str]:
    """
    Format a single pheromone event from Redis Stream into human-readable text.

    This is the NEW implementation for Redis Streams query approach.
    Handles missing keys gracefully per Gemini's recommendation.

    Args:
        event_data: Raw event data from Redis Stream (dict)
                   Expected keys: agent_id, type, location

    Returns:
        Formatted string like "Sage editing config.py" or None if malformed

    Examples:
        >>> format_activity({'agent_id': 'sage-386', 'type': 'WORKING', 'location': 'file:src/config.py'})
        'Sage-386 editing config.py'

        >>> format_activity({'agent_id': 'cascade-623', 'type': 'SUCCESS', 'location': 'task:42'})
        'Cascade-623 completed task:42'

        >>> format_activity({'agent_id': 'broken'})  # Missing keys
        None
    """
    try:
        # Validate required fields with defaults
        agent_id = event_data.get('agent_id')
        ptype = event_data.get('type')
        location = event_data.get('location')

        # Strict validation: require agent_id and location
        if not agent_id or not location:
            log.debug(f"Skipping malformed event: missing agent_id or location: {event_data}")
            return None

        # Type validation
        if not isinstance(agent_id, str) or not isinstance(location, str):
            log.debug(f"Skipping event with non-string fields: {event_data}")
            return None

        # Default type if missing (shouldn't happen but be defensive)
        if not ptype:
            ptype = 'UNKNOWN'

        # Format agent name using helper (CASCADE will implement)
        agent_name = _format_agent_name(agent_id)

        # Extract resource name using helper (CASCADE will implement)
        resource_name = _extract_resource_name(location)

        # Map pheromone types to verbs (from config)
        from presence_injector_config import PHEROMONE_VERBS

        verb = PHEROMONE_VERBS.get(ptype, PHEROMONE_VERBS.get('UNKNOWN', 'at'))

        # Build formatted activity string
        activity = f"{agent_name} {verb} {resource_name}"

        log.debug(f"Formatted activity: {activity}")
        return activity

    except KeyError as e:
        log.warning(f"Missing expected key in event data: {e}")
        return None
    except ValueError as e:
        log.warning(f"Value error formatting activity: {e}")
        return None
    except Exception as e:
        log.error(f"Unexpected error formatting activity: {e}", exc_info=True)
        return None


# ============================================================================
# HELPER FUNCTIONS (CASCADE will implement)
# ============================================================================

def _format_agent_name(agent_id: str) -> str:
    """
    Extract human-readable name from agent ID.
    Implementation based on CASCADE's specification.

    Examples:
        "claude-instance-3" -> "Instance-3"
        "sage-386" -> "Sage-386"
        "lyra" -> "Lyra"
        "" -> "Unknown"
        None -> "Unknown"

    Args:
        agent_id: Raw agent identifier string

    Returns:
        Formatted agent name suitable for display
    """
    from presence_injector_config import DEFAULT_AGENT_NAME, MAX_AGENT_NAME_LENGTH

    # Handle None and empty strings
    if not agent_id or not isinstance(agent_id, str):
        return DEFAULT_AGENT_NAME

    # Trim whitespace
    agent_id = agent_id.strip()
    if not agent_id:
        return DEFAULT_AGENT_NAME

    try:
        # Handle compound names (with hyphens)
        if '-' in agent_id:
            parts = agent_id.split('-')
            # Filter out empty parts
            parts = [p for p in parts if p]

            if len(parts) >= 2:
                # Take last two significant parts
                # e.g., "claude-instance-3" -> "Instance-3"
                name = f"{parts[-2].capitalize()}-{parts[-1]}"
            elif len(parts) == 1:
                # Only one part after split
                name = parts[0].capitalize()
            else:
                # Weird edge case
                return DEFAULT_AGENT_NAME
        else:
            # Simple name without hyphens
            name = agent_id.capitalize()

        # Truncate if too long
        if len(name) > MAX_AGENT_NAME_LENGTH:
            name = name[:MAX_AGENT_NAME_LENGTH - 3] + '...'

        return name

    except Exception as e:
        log.warning(f"Failed to format agent name '{agent_id}': {e}")
        return DEFAULT_AGENT_NAME


def _extract_resource_name(location: str) -> str:
    """
    Extract resource name from location string.
    Implementation based on CASCADE's specification.

    Examples:
        "file:src/database.py" -> "database.py"
        "task:42" -> "task:42"
        "very_long_filename_that_exceeds_limit.py" -> "very_long_filename_t..."
        "" -> "unknown"
        None -> "unknown"

    Args:
        location: Location string in format "type:path"

    Returns:
        Concise resource name (max 20 chars)
    """
    from presence_injector_config import (
        DEFAULT_RESOURCE_NAME,
        MAX_RESOURCE_NAME_LENGTH,
        TRUNCATION_SUFFIX
    )

    # Handle None and empty strings
    if not location or not isinstance(location, str):
        return DEFAULT_RESOURCE_NAME

    # Trim whitespace
    location = location.strip()
    if not location:
        return DEFAULT_RESOURCE_NAME

    try:
        # Check if location has type:path format
        if ':' not in location:
            # No separator, treat as plain resource name
            name = location[:MAX_RESOURCE_NAME_LENGTH]
            if len(location) > MAX_RESOURCE_NAME_LENGTH:
                name = name[:MAX_RESOURCE_NAME_LENGTH - len(TRUNCATION_SUFFIX)] + TRUNCATION_SUFFIX
            return name

        # Split into type and path
        resource_type, resource_path = location.split(':', 1)

        if resource_type == 'file':
            # Extract filename from path (handle both / and \ separators)
            # Split by both separators and take last part
            filename = resource_path.replace('\\', '/').split('/')[-1]

            # Truncate long filenames
            if len(filename) > MAX_RESOURCE_NAME_LENGTH:
                truncate_len = MAX_RESOURCE_NAME_LENGTH - len(TRUNCATION_SUFFIX)
                filename = filename[:truncate_len] + TRUNCATION_SUFFIX

            return filename if filename else DEFAULT_RESOURCE_NAME

        else:
            # For tasks and other types, show type:shortened_path
            # e.g., "task:42" or "project:12345..."
            max_path_length = MAX_RESOURCE_NAME_LENGTH - len(resource_type) - 1  # -1 for colon

            if len(resource_path) > max_path_length:
                resource_path = resource_path[:max_path_length - len(TRUNCATION_SUFFIX)] + TRUNCATION_SUFFIX

            return f"{resource_type}:{resource_path}"

    except Exception as e:
        log.warning(f"Failed to extract resource name from '{location}': {e}")
        return DEFAULT_RESOURCE_NAME


# ============================================================================
# MODULE INFO
# ============================================================================

__version__ = "1.0.0-streams"
__author__ = "Lyra-601 (Team Lead)"
__status__ = "Phase 1 - In Progress"

log.info(f"Presence Injector Streams module loaded (version {__version__})")


# ============================================================================
# REDIS STREAMS QUERY (Lyra - Phase 2)
# ============================================================================

def get_team_context_from_stream() -> Optional[str]:
    """
    Query Redis Stream for recent pheromone events and format as awareness context.

    This is the NEW stateless approach that replaces RedisAwarenessListener.
    Queries stigmergy:pheromones:broadcast stream directly.

    Returns:
        Formatted context string like "[Team Activity: Sage editing config.py | Cascade viewing README.md]"
        None if no recent activity or Redis unavailable

    Implementation Notes:
        - Queries last 50 events from stream (configurable)
        - Filters to 30-second window
        - Skips own activities
        - Returns top 3 formatted activities
        - Graceful degradation on Redis failures
    """
    try:
        import redis
        import time

        # Import configuration (from SAGE's work)
        from presence_injector_config import (
            STREAM_NAME,
            RECENT_EVENTS_COUNT,
            AWARENESS_WINDOW_MS,
            MAX_EVENTS_TO_PROCESS,
            MAX_ACTIVITIES_TO_SHOW,
            REDIS_TIMEOUT_SECONDS,
            CONTEXT_PREFIX,
            CONTEXT_SUFFIX,
            ACTIVITY_SEPARATOR
        )

        # Get Redis connection from environment
        # Note: Our Redis Streams instance is on port 12963
        redis_url = os.getenv('REDIS_URL') or os.getenv('REDIS_STREAMS_URL', 'redis://localhost:12963/0')

        # Create Redis client with timeout
        r = redis.from_url(
            redis_url,
            socket_connect_timeout=REDIS_TIMEOUT_SECONDS,
            socket_timeout=REDIS_TIMEOUT_SECONDS,
            decode_responses=True  # Auto-decode bytes to strings
        )

        # Query last N events from stream
        # XREVRANGE returns events in reverse chronological order (newest first)
        log.debug(f"Querying stream {STREAM_NAME} for last {RECENT_EVENTS_COUNT} events")
        events = r.xrevrange(STREAM_NAME, count=RECENT_EVENTS_COUNT)

        if not events:
            log.debug("No events in stream")
            return None

        # Filter and format events (Phase 3 refactor: use canonical identity)
        now_ms = time.time() * 1000  # Convert to milliseconds
        current_agent = get_ai_id()

        activities = []

        for msg_id, event_data in events[:MAX_EVENTS_TO_PROCESS]:
            # Parse timestamp from Redis message ID (format: "timestamp-sequence")
            try:
                # Redis message IDs are strings like "1730419200000-0"
                timestamp_ms = int(msg_id.split('-')[0])
            except (ValueError, AttributeError, IndexError) as e:
                log.debug(f"Malformed message ID {msg_id}: {e}")
                continue

            # Filter to time window (30 seconds)
            age_ms = now_ms - timestamp_ms
            if age_ms > AWARENESS_WINDOW_MS:
                # Stream is sorted newest-first, so we can break early
                log.debug(f"Event {msg_id} outside window ({age_ms}ms old), stopping")
                break

            # Skip own activities (self-filtering)
            event_agent = event_data.get('agent_id')
            if event_agent == current_agent:
                log.debug(f"Skipping own activity from {event_agent}")
                continue

            # Parse event format: handle both nested (event sourcing) and flat formats
            # Event sourcing format: {'event_type': '...', 'payload': '{"location": "...", "pheromone_type": "..."}'}
            # Flat format: {'agent_id': '...', 'type': '...', 'location': '...'}
            formatted_event = event_data.copy()

            if 'payload' in event_data and 'type' not in event_data:
                # Parse nested event sourcing format
                try:
                    import json as json_mod
                    payload = json_mod.loads(event_data['payload'])

                    # Map event sourcing fields to flat format
                    formatted_event['location'] = payload.get('location')
                    formatted_event['type'] = payload.get('pheromone_type')  # Note: pheromone_type -> type
                    formatted_event['intensity'] = payload.get('intensity')

                except (json_mod.JSONDecodeError, KeyError) as e:
                    log.debug(f"Failed to parse payload for event {msg_id}: {e}")
                    continue

            # Format activity using our robust formatter
            formatted = format_activity(formatted_event)
            if formatted:
                activities.append(formatted)
                log.debug(f"Added activity: {formatted}")

        # Build final context string
        if not activities:
            log.debug("No recent activities after filtering")
            return None

        # Take top N activities
        top_activities = activities[:MAX_ACTIVITIES_TO_SHOW]
        context = ACTIVITY_SEPARATOR.join(top_activities)
        final_context = f"{CONTEXT_PREFIX} {context} {CONTEXT_SUFFIX}"

        log.info(f"Generated awareness context with {len(top_activities)} activities")
        return final_context

    except redis.exceptions.ConnectionError as e:
        log.warning(f"Redis connection failed: {e}")
        return None  # Graceful degradation

    except redis.exceptions.TimeoutError as e:
        log.warning(f"Redis query timeout: {e}")
        return None

    except redis.exceptions.ResponseError as e:
        log.error(f"Redis command error (stream may not exist): {e}")
        return None

    except ImportError as e:
        log.error(f"Redis library not available: {e}")
        return None

    except Exception as e:
        # Catch-all for unexpected errors
        log.error(f"Unexpected error querying awareness stream: {e}", exc_info=True)
        return None
