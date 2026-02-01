#!/usr/bin/env python3
"""
Teambook Awareness Helpers
===========================
Helper functions for awareness injection system.

Queries:
- Unreplied DMs (DMs where we haven't responded to sender)
- New broadcasts since last seen ID

Author: Resonance-403
Date: 2025-11-04
Status: Phase 1 - Awareness Enhancement
"""

import sys
from pathlib import Path
from datetime import datetime, timezone
from typing import List, Dict, Any, Optional
import logging

log = logging.getLogger(__name__)

# Fix import path
sys.path.insert(0, str(Path(__file__).parent))

# Import teambook utilities
try:
    from .teambook_shared import format_time_compact, get_current_ai_id
    from .teambook_utils import get_db_conn
    from .teambook_dm_cache import cache_dm, get_all_cached_dms_to_ai, sync_recent_dms_to_cache
except ImportError:
    from teambook_shared import format_time_compact, get_current_ai_id
    from teambook_utils import get_db_conn
    try:
        from teambook_dm_cache import cache_dm, get_all_cached_dms_to_ai, sync_recent_dms_to_cache
    except ImportError:
        # DM cache not available yet
        cache_dm = None
        get_all_cached_dms_to_ai = None
        sync_recent_dms_to_cache = None
    try:
        from teambook_vote_tracking import get_my_pending_votes, format_vote_awareness
    except ImportError:
        # Vote tracking not available yet
        get_my_pending_votes = None
        format_vote_awareness = None


def get_unreplied_dms(limit: int = 10, use_cache: bool = True) -> List[Dict[str, Any]]:
    """
    Get DMs sent to this AI that haven't been replied to yet.

    A DM is considered "unreplied" if:
    1. It's addressed to this AI
    2. This AI hasn't sent a DM back to the sender since receiving it
    3. It's not expired

    Args:
        limit: Maximum number of unreplied DMs to return (default: 10)
        use_cache: Whether to use DM cache if available (default: True)

    Returns:
        List of dicts with keys: id, from_ai, content, created, time_ago
        Sorted by created DESC (newest first)
    """
    try:
        current_ai_id = get_current_ai_id()
        # If cache available and enabled, try cache first
        if use_cache and get_all_cached_dms_to_ai is not None:
            try:
                cached_dms = get_all_cached_dms_to_ai(current_ai_id, limit=limit)
                if cached_dms:
                    # Format cached DMs to match expected output
                    results = []
                    for dm in cached_dms:
                        results.append({
                            'id': dm.get('original_msg_id', 0),
                            'from': dm['from_ai'],
                            'content': dm['content'],
                            'created': dm['created'],
                            'time_ago': format_time_compact(dm['created'])
                        })
                    log.debug(f"Retrieved {len(results)} DMs from cache")
                    return results[:limit]
            except Exception as cache_error:
                log.debug(f"Cache lookup failed, falling back to database: {cache_error}")

        # Fall back to database query
        with get_db_conn() as conn:
            # Query for DMs sent to this AI where we haven't replied to sender
            query = '''
                SELECT
                    m.id,
                    m.from_ai,
                    m.content,
                    m.created
                FROM messages m
                WHERE
                    LOWER(m.to_ai) = LOWER(?)
                    AND m.expires_at > ?
                    AND m.channel IS NULL  -- DMs have no channel
                    -- Check if we've replied (sent DM back to this sender after this message)
                    AND NOT EXISTS (
                        SELECT 1
                        FROM messages reply
                        WHERE LOWER(reply.from_ai) = LOWER(?)
                          AND LOWER(reply.to_ai) = LOWER(m.from_ai)
                          AND reply.created > m.created
                          AND reply.channel IS NULL
                    )
                ORDER BY m.created DESC
                LIMIT ?
            '''

            rows = conn.execute(query, [
                current_ai_id,
                datetime.now(timezone.utc),
                current_ai_id,  # For the NOT EXISTS subquery
                limit
            ]).fetchall()

            if not rows:
                return []

            # Format results
            results = []
            for msg_id, from_ai, content, created in rows:
                results.append({
                    'id': msg_id,
                    'from': from_ai,
                    'content': content,
                    'created': created,
                    'time_ago': format_time_compact(created)
                })

            log.debug(f"Found {len(results)} unreplied DMs")
            return results

    except Exception as e:
        log.error(f"Failed to query unreplied DMs: {e}")
        return []


def get_broadcasts_since(last_seen_id: int, channel: str = "general", limit: int = 10, max_age_hours: int = 4) -> List[Dict[str, Any]]:
    """
    Get broadcasts posted since the last seen broadcast ID.

    Args:
        last_seen_id: Last broadcast ID that was shown to AI (0 = show recent ones)
        channel: Channel name (default: "general")
        limit: Maximum number of broadcasts to return (default: 10)
        max_age_hours: Maximum age of broadcasts in hours (default: 4, prevents showing very old messages)

    Returns:
        List of dicts with keys: id, from_ai, content, created, time_ago
        Sorted by created ASC (oldest first, so they're shown in order)
    """
    try:
        from datetime import timedelta

        with get_db_conn() as conn:
            # Calculate cutoff time (don't show broadcasts older than max_age_hours)
            cutoff_time = datetime.now(timezone.utc) - timedelta(hours=max_age_hours)

            # Query for broadcasts newer than last_seen_id AND not too old
            query = '''
                SELECT
                    id,
                    from_ai,
                    content,
                    created
                FROM messages
                WHERE
                    LOWER(channel) = LOWER(?)
                    AND id > ?
                    AND expires_at > ?
                    AND created > ?  -- Age filter: only show recent broadcasts
                    AND to_ai IS NULL  -- Broadcasts have no to_ai
                ORDER BY created ASC  -- Oldest first (chronological order)
                LIMIT ?
            '''

            rows = conn.execute(query, [
                channel,
                last_seen_id,
                datetime.now(timezone.utc),
                cutoff_time,  # Age filter parameter
                limit
            ]).fetchall()

            if not rows:
                return []

            # Format results
            results = []
            for msg_id, from_ai, content, created in rows:
                results.append({
                    'id': msg_id,
                    'from': from_ai,
                    'content': content,
                    'created': created,
                    'time_ago': format_time_compact(created)
                })

            log.debug(f"Found {len(results)} new broadcasts since ID {last_seen_id}")
            return results

    except Exception as e:
        log.error(f"Failed to query broadcasts since {last_seen_id}: {e}")
        return []


def get_most_recent_broadcast_id(channel: str = "general") -> int:
    """
    Get the ID of the most recent broadcast in a channel.

    Used to initialize state tracking when starting fresh.

    Args:
        channel: Channel name (default: "general")

    Returns:
        Most recent broadcast ID, or 0 if no broadcasts exist
    """
    try:
        with get_db_conn() as conn:
            query = '''
                SELECT MAX(id)
                FROM messages
                WHERE LOWER(channel) = LOWER(?)
                  AND to_ai IS NULL
                  AND expires_at > ?
            '''

            result = conn.execute(query, [
                channel,
                datetime.now(timezone.utc)
            ]).fetchone()

            if result and result[0]:
                return result[0]
            return 0

    except Exception as e:
        log.error(f"Failed to get most recent broadcast ID: {e}")
        return 0


def get_active_detangle_sessions() -> List[Dict[str, Any]]:
    """
    Get active detangle sessions where this AI is a participant.

    Returns:
        List of dicts with keys: session_id, other_ai, topic, current_turn,
        my_turns_left, other_turns_left, created, expires
    """
    try:
        current_ai_id = get_current_ai_id()
        with get_db_conn() as conn:
            now = datetime.now(timezone.utc)

            # Query for active sessions where this AI is participant
            query = '''
                SELECT
                    id,
                    ai_1,
                    ai_2,
                    topic,
                    current_turn,
                    turns_remaining_1,
                    turns_remaining_2,
                    created,
                    expires
                FROM detangle_sessions
                WHERE state = 'ACTIVE'
                  AND expires > ?
                  AND (LOWER(ai_1) = LOWER(?) OR LOWER(ai_2) = LOWER(?))
                ORDER BY created DESC
            '''

            rows = conn.execute(query, [
                now,
                current_ai_id,
                current_ai_id
            ]).fetchall()

            if not rows:
                return []

            # Format results
            results = []
            for session_id, ai_1, ai_2, topic, current_turn, turns_remaining_1, turns_remaining_2, created, expires in rows:
                # Determine who is "other AI" and turn counts
                if ai_1.lower() == current_ai_id.lower():
                    other_ai = ai_2
                    my_turns_left = turns_remaining_1
                    other_turns_left = turns_remaining_2
                else:
                    other_ai = ai_1
                    my_turns_left = turns_remaining_2
                    other_turns_left = turns_remaining_1

                # Check if it's my turn
                is_my_turn = current_turn.lower() == current_ai_id.lower()

                results.append({
                    'session_id': session_id,
                    'other_ai': other_ai,
                    'topic': topic,
                    'is_my_turn': is_my_turn,
                    'my_turns_left': my_turns_left,
                    'other_turns_left': other_turns_left,
                    'created': created,
                    'expires': expires,
                    'time_ago': format_time_compact(created)
                })

            log.debug(f"Found {len(results)} active detangle sessions")
            return results

    except Exception as e:
        log.error(f"Failed to query active detangle sessions: {e}")
        return []


# Cache for timestamp suppression (only show when minute changes)
_last_timestamp_minute = None

def format_awareness_context(unreplied_dms: List[Dict], new_broadcasts: List[Dict], activity: Optional[str] = None, votes: Optional[str] = None, detangle_sessions: Optional[List[Dict]] = None, max_content_length: int = 400) -> str:
    """
    Format unreplied DMs, new broadcasts, votes, detangle sessions, and activity into awareness context string.

    Token-optimized: Uses |HEADER| format instead of box drawing (saves ~35 tokens per injection).
    Timestamp suppression: Only shows time when minute changes (saves ~5-40 repetitions per session).

    Args:
        unreplied_dms: List of unreplied DM dicts from get_unreplied_dms()
        new_broadcasts: List of new broadcast dicts from get_broadcasts_since()
        activity: Optional file activity string from pheromone system
        votes: Optional vote status string from format_vote_awareness()
        detangle_sessions: Optional list of active detangle sessions from get_active_detangle_sessions()
        max_content_length: Maximum length for message content before truncation (default: 400)

    Returns:
        Formatted awareness context suitable for injection, or None if no content
    """
    global _last_timestamp_minute
    parts = []

    # Token-optimized header (|HEADER| format instead of box drawing)
    # Saves ~35 tokens per injection while remaining clear and readable
    parts.append("|🔴 NEW TEAM INFORMATION|")

    # Timestamp: Only show when minute changes (saves repeated UTC spam)
    now = datetime.now(timezone.utc)
    current_minute = (now.hour, now.minute)

    if _last_timestamp_minute != current_minute:
        timestamp = now.strftime("[%I:%M%p-UTC]")
        parts.append(timestamp)
        _last_timestamp_minute = current_minute

    # 1. Active votes (highest priority - action required)
    if votes:
        parts.append(votes)

    # 2. Unreplied DMs (critical - always show full content)
    if unreplied_dms:
        dm_lines = [f"📩 Unreplied DMs ({len(unreplied_dms)}):"]
        for dm in unreplied_dms:
            # Show FULL content - no truncation (user requirement)
            content = dm['content']
            dm_lines.append(f"  - {dm['from']}: \"{content}\"")
        parts.append('\n'.join(dm_lines))

    # 3. New broadcasts (recent team updates)
    if new_broadcasts:
        bc_lines = [f"📢 New Broadcasts ({len(new_broadcasts)}):"]
        for bc in new_broadcasts:
            # Show FULL content - no truncation (user requirement)
            content = bc['content']
            bc_lines.append(f"  - {bc['from']} [{bc['time_ago']}]: \"{content}\"")
        parts.append('\n'.join(bc_lines))

    # 4. Active detangle sessions (coordination awareness)
    if detangle_sessions:
        detangle_lines = [f"🎯 Active Detangle Sessions ({len(detangle_sessions)}):"]
        for session in detangle_sessions:
            turn_indicator = "🔔 YOUR TURN" if session['is_my_turn'] else f"⏳ Waiting for {session['other_ai']}"
            turns_info = f"{session['my_turns_left']} turns left"
            detangle_lines.append(
                f"  - Session #{session['session_id']} with {session['other_ai']} | {turn_indicator} ({turns_info})"
            )
            detangle_lines.append(f"    Topic: {session['topic']}")
        parts.append('\n'.join(detangle_lines))

    # 5. File activity (current pheromones)
    if activity:
        parts.append(f"🔧 Activity: {activity}")

    # If no content besides header and timestamp, return None
    if len(parts) == 2:  # Only prominent header + timestamp
        return None

    # Combine all parts with blank line separator
    return '\n\n'.join(parts)


if __name__ == '__main__':
    # Test/debug mode
    import json
    import sys

    # Fix Windows console encoding for emojis
    if sys.platform == 'win32':
        sys.stdout.reconfigure(encoding='utf-8')

    print("Testing awareness helpers...\n")

    print("1. Unreplied DMs:")
    dms = get_unreplied_dms(limit=5)
    if dms:
        for dm in dms:
            print(f"  - ID {dm['id']} from {dm['from']}: {dm['content'][:50]}...")
    else:
        print("  (none)")

    print("\n2. New broadcasts since ID 0:")
    broadcasts = get_broadcasts_since(last_seen_id=0, limit=5)
    if broadcasts:
        for bc in broadcasts:
            print(f"  - ID {bc['id']} from {bc['from']}: {bc['content'][:50]}...")
    else:
        print("  (none)")

    print("\n3. Most recent broadcast ID:")
    print(f"  {get_most_recent_broadcast_id()}")

    print("\n4. Active detangle sessions:")
    detangle_sessions = get_active_detangle_sessions()
    if detangle_sessions:
        for session in detangle_sessions:
            print(f"  - Session #{session['session_id']} with {session['other_ai']}")
            print(f"    Topic: {session['topic']}")
            print(f"    Your turn: {session['is_my_turn']}, Turns left: {session['my_turns_left']}")
    else:
        print("  (none)")

    print("\n5. Formatted context:")
    context = format_awareness_context(
        dms,
        broadcasts,
        activity="Sage editing config.py",
        detangle_sessions=detangle_sessions
    )
    print(context if context else "  (no context)")
