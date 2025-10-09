#!/usr/bin/env python3
"""
TEAMBOOK EVENTS v1.0.0 - REAL-TIME AWARENESS SYSTEM
====================================================
Watch items, receive notifications automatically. Zero complexity for AIs.

Security Features:
- Max 50 watches per AI (prevents resource hoarding)
- Event content sanitized (no secrets leaked)
- Only see events for watched items
- Auto-cleanup after 7 days
- Rate limiting on queries (100/min)

Performance Features:
- Indexed queries on hot paths
- In-memory active watch cache
- Batch event delivery
- Periodic cleanup
"""

import time
from datetime import datetime, timedelta, timezone
from typing import Dict, List, Optional, Tuple, Any
from collections import defaultdict
import logging
import json

from teambook_shared import (
    CURRENT_AI_ID, CURRENT_TEAMBOOK, OUTPUT_FORMAT,
    pipe_escape, format_time_compact, clean_text
)

from teambook_storage import get_db_conn, log_operation_to_db

# ============= SECURITY LIMITS =============

MAX_WATCHES_PER_AI = 50  # Prevent resource hoarding
MAX_EVENT_QUERY_RATE = 100  # Per minute per AI
EVENT_RETENTION_DAYS = 7  # Auto-cleanup old events
MAX_EVENT_CONTENT = 500  # Bytes, prevent bloat
MAX_EVENTS_PER_QUERY = 1000

# Rate limiting state
_event_query_limiter = defaultdict(list)  # ai_id -> [timestamps]

# Active watch cache (performance)
_active_watches_cache = {}  # (ai_id, item_type, item_id) -> watch_id
_cache_timestamp = None
_cache_ttl = 60  # Refresh cache every 60 seconds

# ============= INPUT VALIDATION =============

def sanitize_item_type(item_type: str) -> Optional[str]:
    """
    Sanitize item type - SECURITY CRITICAL

    Returns None if invalid, sanitized string if valid.
    """
    if not item_type:
        return None

    item_type = str(item_type).strip().lower()

    # Whitelist of valid item types
    valid_types = {
        'note', 'lock', 'channel', 'evolution',
        'contribution', 'task', 'message'
    }

    if item_type not in valid_types:
        return None

    return item_type

def sanitize_item_id(item_id: Any) -> Optional[str]:
    """Sanitize item ID"""
    if item_id is None:
        return None

    item_id = str(item_id).strip()

    if len(item_id) > 200:
        return None

    return item_id

def sanitize_event_types(event_types: Any) -> Optional[List[str]]:
    """
    Sanitize event type filter.

    Returns None for 'all events', list of valid types otherwise.
    """
    if not event_types or event_types == 'all':
        return None  # Watch all events

    if isinstance(event_types, str):
        event_types = [event_types]

    if not isinstance(event_types, list):
        return None

    # Whitelist of valid event types
    valid_events = {
        'created', 'edited', 'deleted', 'pinned', 'unpinned',
        'claimed', 'released', 'assigned', 'completed',
        'locked', 'unlocked', 'sent', 'received',
        'contributed', 'synthesized', 'ranked', 'voted'
    }

    sanitized = []
    for et in event_types:
        et = str(et).strip().lower()
        if et in valid_events:
            sanitized.append(et)

    return sanitized if sanitized else None

def check_event_query_rate(ai_id: str) -> Tuple[bool, int]:
    """
    Check if AI is within event query rate limits.

    Returns (allowed, remaining_quota)
    """
    now = time.time()
    minute_ago = now - 60

    # Clean old timestamps
    _event_query_limiter[ai_id] = [t for t in _event_query_limiter[ai_id] if t > minute_ago]

    current_count = len(_event_query_limiter[ai_id])
    remaining = MAX_EVENT_QUERY_RATE - current_count

    if current_count >= MAX_EVENT_QUERY_RATE:
        return False, 0

    _event_query_limiter[ai_id].append(now)
    return True, remaining - 1

# ============= DATABASE INITIALIZATION =============

def init_events_tables(conn):
    """Initialize event system tables"""

    # Create sequences for auto-increment
    try:
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_watches')
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_events')
    except Exception:
        pass  # Sequences might already exist

    # Watches table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS watches (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_watches'),
            ai_id VARCHAR(100) NOT NULL,
            item_type VARCHAR(20) NOT NULL,
            item_id VARCHAR(200) NOT NULL,
            event_types TEXT,
            created_at TIMESTAMPTZ NOT NULL,
            last_activity TIMESTAMPTZ,
            teambook_name VARCHAR(50),
            UNIQUE(ai_id, item_type, item_id)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_watches_ai ON watches(ai_id, last_activity)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_watches_item ON watches(item_type, item_id)')

    # Events table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_events'),
            item_type VARCHAR(20) NOT NULL,
            item_id VARCHAR(200) NOT NULL,
            event_type VARCHAR(50) NOT NULL,
            actor_ai_id VARCHAR(100) NOT NULL,
            summary TEXT,
            created_at TIMESTAMPTZ NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL,
            teambook_name VARCHAR(50),
            metadata TEXT
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_events_item ON events(item_type, item_id, created_at DESC)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_events_time ON events(created_at DESC)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_events_expires ON events(expires_at)')

    # Event deliveries (who has seen which events)
    conn.execute('''
        CREATE TABLE IF NOT EXISTS event_deliveries (
            event_id INTEGER NOT NULL,
            ai_id VARCHAR(100) NOT NULL,
            seen BOOLEAN DEFAULT FALSE,
            delivered_at TIMESTAMPTZ,
            PRIMARY KEY(event_id, ai_id)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_deliveries_ai ON event_deliveries(ai_id, seen)')

    conn.commit()

def cleanup_expired_events(conn):
    """Remove expired events and inactive watches"""
    try:
        now = datetime.now(timezone.utc)

        # Delete expired events
        expired = conn.execute(
            'DELETE FROM events WHERE expires_at < ? RETURNING id',
            [now]
        ).fetchall()

        if expired:
            # Clean up deliveries for deleted events
            event_ids = [e[0] for e in expired]
            placeholders = ','.join(['?'] * len(event_ids))
            conn.execute(f'DELETE FROM event_deliveries WHERE event_id IN ({placeholders})', event_ids)

            conn.commit()
            logging.info(f"Cleaned up {len(expired)} expired events")

        # Clean up inactive watches (24h no activity)
        inactive_threshold = now - timedelta(hours=24)
        inactive = conn.execute(
            'DELETE FROM watches WHERE last_activity < ? RETURNING id',
            [inactive_threshold]
        ).fetchall()

        if inactive:
            conn.commit()
            logging.info(f"Cleaned up {len(inactive)} inactive watches")

    except Exception as e:
        logging.error(f"Event cleanup error: {e}")

# ============= CACHE MANAGEMENT =============

def refresh_watch_cache(conn):
    """Refresh in-memory cache of active watches"""
    global _active_watches_cache, _cache_timestamp

    now = time.time()

    # Only refresh if cache is stale
    if _cache_timestamp and (now - _cache_timestamp) < _cache_ttl:
        return

    try:
        watches = conn.execute('''
            SELECT id, ai_id, item_type, item_id
            FROM watches
            WHERE last_activity > ?
        ''', [datetime.now(timezone.utc) - timedelta(hours=24)]).fetchall()

        _active_watches_cache = {
            (ai_id, item_type, item_id): watch_id
            for watch_id, ai_id, item_type, item_id in watches
        }

        _cache_timestamp = now

    except Exception as e:
        logging.error(f"Cache refresh error: {e}")

# ============= EVENT EMISSION (INTERNAL) =============

def emit_event(item_type: str, item_id: str, event_type: str,
               summary: str = None, metadata: Dict = None) -> Optional[int]:
    """
    Emit an event (internal function, called by other modules).

    This creates the event and queues it for delivery to watchers.
    AIs never call this directly.
    """
    try:
        item_type = sanitize_item_type(item_type)
        item_id = sanitize_item_id(item_id)

        if not item_type or not item_id:
            return None

        # Sanitize summary
        if summary:
            summary = clean_text(summary)
            if len(summary) > MAX_EVENT_CONTENT:
                summary = summary[:MAX_EVENT_CONTENT]

        # Calculate expiration
        expires_at = datetime.now(timezone.utc) + timedelta(days=EVENT_RETENTION_DAYS)

        with get_db_conn() as conn:
            init_events_tables(conn)

            # Create event
            cursor = conn.execute('''
                INSERT INTO events (
                    item_type, item_id, event_type, actor_ai_id,
                    summary, created_at, expires_at, teambook_name, metadata
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                RETURNING id
            ''', [
                item_type, item_id, event_type, CURRENT_AI_ID,
                summary, datetime.now(timezone.utc), expires_at,
                CURRENT_TEAMBOOK, json.dumps(metadata) if metadata else None
            ])

            event_id = cursor.fetchone()[0]

            # Find watchers
            watchers = conn.execute('''
                SELECT ai_id, event_types
                FROM watches
                WHERE item_type = ? AND item_id = ?
            ''', [item_type, item_id]).fetchall()

            # Create delivery records
            for watcher_ai, event_types_json in watchers:
                # Check if watcher wants this event type
                if event_types_json:
                    wanted_types = json.loads(event_types_json)
                    if event_type not in wanted_types:
                        continue

                conn.execute('''
                    INSERT INTO event_deliveries (event_id, ai_id, seen)
                    VALUES (?, ?, FALSE)
                ''', [event_id, watcher_ai])

            # Push to WebSocket clients if streaming available (Phase 2.5)
            try:
                from teambook_streaming import push_event_to_watchers
                push_event_to_watchers(event_id, item_type, item_id, {
                    'event_type': event_type,
                    'summary': summary,
                    'actor_ai_id': CURRENT_AI_ID
                })
            except ImportError:
                pass  # Streaming not available, use polling

            # Periodic cleanup (10% chance)
            import random
            if random.random() < 0.1:
                cleanup_expired_events(conn)

            return event_id

    except Exception as e:
        logging.error(f"Error emitting event: {e}")
        return None

# ============= CORE FUNCTIONS =============

def watch(item_id: Any = None, item_type: str = "note", event_types: Any = None, **kwargs) -> Dict:
    """
    Watch an item for changes.

    Example:
        watch(note_id=42)
        watch(note_id=42, event_types=["edited", "deleted"])
        watch(item_id="auth.py", item_type="lock")
    """
    try:
        # Handle note_id, lock_id, etc. shortcuts
        if 'note_id' in kwargs:
            item_id = kwargs['note_id']
            item_type = 'note'
        elif 'lock_id' in kwargs:
            item_id = kwargs['lock_id']
            item_type = 'lock'
        elif 'channel' in kwargs:
            item_id = kwargs['channel']
            item_type = 'channel'
        else:
            item_id = kwargs.get('item_id', item_id)
            item_type = kwargs.get('item_type', item_type)

        # Sanitize
        item_type = sanitize_item_type(item_type)
        item_id = sanitize_item_id(item_id)
        event_types_list = sanitize_event_types(kwargs.get('event_types', event_types))

        if not item_type or not item_id:
            return {"error": "invalid_item"}

        with get_db_conn() as conn:
            init_events_tables(conn)

            # Check watch limit
            watch_count = conn.execute(
                'SELECT COUNT(*) FROM watches WHERE ai_id = ?',
                [CURRENT_AI_ID]
            ).fetchone()[0]

            if watch_count >= MAX_WATCHES_PER_AI:
                return {"error": f"watch_limit|max:{MAX_WATCHES_PER_AI}"}

            # Insert or update watch
            now = datetime.now(timezone.utc)
            event_types_json = json.dumps(event_types_list) if event_types_list else None

            conn.execute('''
                INSERT INTO watches (
                    ai_id, item_type, item_id, event_types,
                    created_at, last_activity, teambook_name
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(ai_id, item_type, item_id) DO UPDATE SET
                    event_types = excluded.event_types,
                    last_activity = excluded.last_activity
            ''', [CURRENT_AI_ID, item_type, item_id, event_types_json,
                  now, now, CURRENT_TEAMBOOK])

            conn.commit()

            # Invalidate cache
            global _cache_timestamp
            _cache_timestamp = None

        # Subscribe WebSocket if AI has active connection (Phase 2.5)
        try:
            from teambook_streaming import get_active_connection, subscribe_websocket
            conn_id = get_active_connection(CURRENT_AI_ID)
            if conn_id:
                subscribe_websocket(conn_id, item_type, item_id, event_types_list)
        except ImportError:
            pass  # Streaming not available

        log_operation_to_db('watch')

        event_count = len(event_types_list) if event_types_list else 'all'
        result = f"{item_type}:{item_id}|events:{event_count}"

        return {"watching": result}

    except Exception as e:
        logging.error(f"Watch error: {e}")
        return {"error": "watch_failed"}

def unwatch(item_id: Any = None, item_type: str = "note", **kwargs) -> Dict:
    """
    Stop watching an item.

    Example:
        unwatch(note_id=42)
        unwatch(item_id="auth.py", item_type="lock")
    """
    try:
        # Handle shortcuts
        if 'note_id' in kwargs:
            item_id = kwargs['note_id']
            item_type = 'note'
        elif 'lock_id' in kwargs:
            item_id = kwargs['lock_id']
            item_type = 'lock'
        else:
            item_id = kwargs.get('item_id', item_id)
            item_type = kwargs.get('item_type', item_type)

        item_type = sanitize_item_type(item_type)
        item_id = sanitize_item_id(item_id)

        if not item_type or not item_id:
            return {"error": "invalid_item"}

        with get_db_conn() as conn:
            init_events_tables(conn)

            result = conn.execute('''
                DELETE FROM watches
                WHERE ai_id = ? AND item_type = ? AND item_id = ?
            ''', [CURRENT_AI_ID, item_type, item_id])

            if result.rowcount == 0:
                return {"error": "not_watching"}

            # Invalidate cache
            global _cache_timestamp
            _cache_timestamp = None

        log_operation_to_db('unwatch')

        return {"unwatched": f"{item_type}:{item_id}"}

    except Exception as e:
        logging.error(f"Unwatch error: {e}")
        return {"error": "unwatch_failed"}

def get_events(since: Any = None, limit: int = 20, mark_seen: bool = True, **kwargs) -> Dict:
    """
    Get events for items you're watching.

    Example:
        get_events()  # Recent events
        get_events(since="5m")  # Last 5 minutes
        get_events(since=1234567890)  # Unix timestamp
        get_events(limit=50)
    """
    try:
        # Rate limiting
        allowed, remaining = check_event_query_rate(CURRENT_AI_ID)
        if not allowed:
            return {"error": "rate_limit|wait_60s"}

        # Parse 'since' parameter
        since_dt = None
        if since:
            if isinstance(since, (int, float)):
                # Unix timestamp
                since_dt = datetime.fromtimestamp(since, tz=timezone.utc)
            elif isinstance(since, str):
                # Parse time strings like "5m", "1h", "2d"
                since = since.strip().lower()
                if since.endswith('m'):
                    minutes = int(since[:-1])
                    since_dt = datetime.now(timezone.utc) - timedelta(minutes=minutes)
                elif since.endswith('h'):
                    hours = int(since[:-1])
                    since_dt = datetime.now(timezone.utc) - timedelta(hours=hours)
                elif since.endswith('d'):
                    days = int(since[:-1])
                    since_dt = datetime.now(timezone.utc) - timedelta(days=days)

        if not since_dt:
            # Default: last 24 hours
            since_dt = datetime.now(timezone.utc) - timedelta(hours=24)

        limit = int(kwargs.get('limit', limit or 20))
        if limit < 1 or limit > MAX_EVENTS_PER_QUERY:
            limit = 20

        mark_seen = bool(kwargs.get('mark_seen', mark_seen if mark_seen is not None else True))

        with get_db_conn() as conn:
            init_events_tables(conn)
            refresh_watch_cache(conn)

            # Get events for watched items
            events = conn.execute('''
                SELECT e.id, e.item_type, e.item_id, e.event_type,
                       e.actor_ai_id, e.summary, e.created_at, d.seen
                FROM events e
                JOIN event_deliveries d ON e.id = d.event_id
                WHERE d.ai_id = ? AND e.created_at > ?
                ORDER BY e.created_at DESC
                LIMIT ?
            ''', [CURRENT_AI_ID, since_dt, limit]).fetchall()

            if not events:
                return {"msg": "no_events"}

            # Mark as seen if requested
            if mark_seen:
                event_ids = [e[0] for e in events]
                placeholders = ','.join(['?'] * len(event_ids))
                conn.execute(f'''
                    UPDATE event_deliveries
                    SET seen = TRUE, delivered_at = ?
                    WHERE event_id IN ({placeholders}) AND ai_id = ?
                ''', [datetime.now(timezone.utc)] + event_ids + [CURRENT_AI_ID])

        log_operation_to_db('get_events')

        # Format events (minimal, pipe-delimited)
        if OUTPUT_FORMAT == 'pipe':
            lines = []
            for event_id, item_type, item_id, event_type, actor, summary, created, seen in events:
                parts = [
                    f"event:{event_id}",
                    f"{item_type}:{item_id}",
                    event_type,
                    actor,
                    format_time_compact(created)
                ]
                if summary:
                    parts.append(summary[:50])  # First 50 chars
                if not seen:
                    parts.append('[NEW]')

                lines.append('|'.join(pipe_escape(p) for p in parts))

            result = {"events": lines}
            if remaining < 10:
                result["quota"] = remaining
            return result
        else:
            formatted = []
            for event_id, item_type, item_id, event_type, actor, summary, created, seen in events:
                formatted.append({
                    'id': event_id,
                    'item': f"{item_type}:{item_id}",
                    'type': event_type,
                    'actor': actor,
                    'time': format_time_compact(created),
                    'summary': summary,
                    'new': not seen
                })
            return {"events": formatted, "quota": remaining}

    except Exception as e:
        logging.error(f"Get events error: {e}")
        return {"error": "get_events_failed"}

def list_watches(**kwargs) -> Dict:
    """List all items you're watching"""
    try:
        with get_db_conn() as conn:
            init_events_tables(conn)

            watches = conn.execute('''
                SELECT item_type, item_id, event_types, created_at
                FROM watches
                WHERE ai_id = ?
                ORDER BY last_activity DESC
            ''', [CURRENT_AI_ID]).fetchall()

        if not watches:
            return {"msg": "no_watches"}

        if OUTPUT_FORMAT == 'pipe':
            lines = []
            for item_type, item_id, event_types_json, created in watches:
                event_count = 'all'
                if event_types_json:
                    event_types = json.loads(event_types_json)
                    event_count = len(event_types)

                parts = [
                    f"{item_type}:{item_id}",
                    f"events:{event_count}",
                    f"watching:{format_time_compact(created)}"
                ]
                lines.append('|'.join(pipe_escape(p) for p in parts))
            return {"watches": lines}
        else:
            formatted = []
            for item_type, item_id, event_types_json, created in watches:
                watch_dict = {
                    'item': f"{item_type}:{item_id}",
                    'watching_since': format_time_compact(created)
                }
                if event_types_json:
                    watch_dict['event_types'] = json.loads(event_types_json)
                formatted.append(watch_dict)
            return {"watches": formatted}

    except Exception as e:
        logging.error(f"List watches error: {e}")
        return {"error": "list_failed"}

def watch_stats(**kwargs) -> Dict:
    """Get watching activity overview"""
    try:
        with get_db_conn() as conn:
            init_events_tables(conn)

            stats = conn.execute('''
                SELECT
                    COUNT(*) as watch_count,
                    (SELECT COUNT(*) FROM event_deliveries WHERE ai_id = ? AND seen = FALSE) as unseen_count
                FROM watches
                WHERE ai_id = ?
            ''', [CURRENT_AI_ID, CURRENT_AI_ID]).fetchone()

            watch_count, unseen_count = stats

            # Get last event time
            last_event = conn.execute('''
                SELECT e.created_at
                FROM events e
                JOIN event_deliveries d ON e.id = d.event_id
                WHERE d.ai_id = ?
                ORDER BY e.created_at DESC
                LIMIT 1
            ''', [CURRENT_AI_ID]).fetchone()

            last_event_time = format_time_compact(last_event[0]) if last_event else "never"

        if OUTPUT_FORMAT == 'pipe':
            parts = [
                f"watching:{watch_count}",
                f"unseen:{unseen_count}",
                f"last:{last_event_time}"
            ]
            return {"stats": '|'.join(parts)}
        else:
            return {
                "watching": watch_count,
                "unseen_events": unseen_count,
                "last_event": last_event_time
            }

    except Exception as e:
        logging.error(f"Watch stats error: {e}")
        return {"error": "stats_failed"}

# ============= TEAMBOOK NOTIFICATION HELPER =============

def get_pending_notifications(ai_id: str = None) -> Dict:
    """
    Get summary of pending events/activity for an AI.
    Called automatically when AI does recall/status.

    Returns compact summary of what's happening.
    """
    try:
        ai_id = ai_id or CURRENT_AI_ID

        with get_db_conn() as conn:
            init_events_tables(conn)

            # Count unseen events
            unseen = conn.execute('''
                SELECT COUNT(*)
                FROM event_deliveries
                WHERE ai_id = ? AND seen = FALSE
            ''', [ai_id]).fetchone()[0]

            if unseen == 0:
                return None  # No notifications

            # Get recent event types
            recent = conn.execute('''
                SELECT e.event_type, COUNT(*) as count
                FROM events e
                JOIN event_deliveries d ON e.id = d.event_id
                WHERE d.ai_id = ? AND d.seen = FALSE
                GROUP BY e.event_type
                ORDER BY count DESC
                LIMIT 3
            ''', [ai_id]).fetchall()

            summary_parts = [f"{count} {event_type}" for event_type, count in recent]
            summary = ", ".join(summary_parts)

            return {
                "unseen": unseen,
                "summary": summary
            }

    except Exception as e:
        logging.error(f"Get notifications error: {e}")
        return None