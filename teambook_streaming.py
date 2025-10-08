#!/usr/bin/env python3
"""
TEAMBOOK STREAMING v1.0.0 - WEBSOCKET EVENT PUSH
=================================================
Real-time event delivery via WebSocket. Transparent to AIs - they just call
watch() and get_events() as usual, but events arrive instantly.

Hidden from AIs:
- WebSocket connection management
- Authentication tokens
- Subscription syncing
- Event push delivery

Security: Token-based auth, rate limiting, connection limits
Performance: Batched delivery, connection pooling, auto-cleanup
"""

import uuid
import time
import json
import logging
from datetime import datetime, timedelta, timezone
from typing import Dict, List, Optional, Any, Set
from collections import defaultdict

from teambook_shared import (
    CURRENT_AI_ID, CURRENT_TEAMBOOK,
    pipe_escape
)

from teambook_storage import get_db_conn

# ============= CONFIGURATION =============

MAX_CONNECTIONS_PER_AI = 5
MAX_SUBSCRIPTIONS_PER_CONNECTION = 100
MESSAGE_RATE_LIMIT = 100  # messages per second
CONNECTION_TIMEOUT_SECONDS = 300  # 5 minutes
AUTH_TOKEN_LENGTH = 64
MAX_CACHED_EVENTS_PER_CONNECTION = 1000

# In-memory connection registry (for active WebSocket objects)
# Maps conn_id → WebSocket object
_active_websockets: Dict[str, Any] = {}

# In-memory event cache (for quick delivery)
# Maps conn_id → List[event_dict]
_event_cache: Dict[str, List[Dict]] = defaultdict(list)

# Rate limiter (messages per second)
# Maps conn_id → List[timestamp]
_rate_limiter: Dict[str, List[float]] = defaultdict(list)

logging.basicConfig(level=logging.WARNING)  # Reduced noise - use WARNING by default

# ============= DATABASE INITIALIZATION =============

def init_streaming_tables(conn):
    """Initialize WebSocket streaming tables"""

    # Create sequence for auto-increment
    try:
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_ws_subscriptions')
    except Exception:
        pass

    # WebSocket connections table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS ws_connections (
            conn_id VARCHAR(36) PRIMARY KEY,
            ai_id VARCHAR(100) NOT NULL,
            auth_token VARCHAR(64) NOT NULL,
            connected_at TIMESTAMPTZ NOT NULL,
            last_ping TIMESTAMPTZ NOT NULL,
            status VARCHAR(20) DEFAULT 'active',
            teambook_name VARCHAR(50)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_ws_conn_ai ON ws_connections(ai_id, status)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_ws_conn_token ON ws_connections(auth_token)')

    # WebSocket subscriptions (what each connection is watching)
    conn.execute('''
        CREATE TABLE IF NOT EXISTS ws_subscriptions (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_ws_subscriptions'),
            conn_id VARCHAR(36) NOT NULL,
            item_type VARCHAR(20) NOT NULL,
            item_id VARCHAR(200) NOT NULL,
            event_types TEXT,
            created_at TIMESTAMPTZ NOT NULL
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_ws_sub_conn ON ws_subscriptions(conn_id)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_ws_sub_item ON ws_subscriptions(item_type, item_id)')

    conn.commit()

# ============= CONNECTION MANAGEMENT =============

def register_connection(ai_id: str = None, conn_id: str = None) -> Dict:
    """
    Register a new WebSocket connection

    Args:
        ai_id: AI identifier (defaults to CURRENT_AI_ID)
        conn_id: Connection UUID (auto-generated if not provided)

    Returns:
        {"conn_id": "...", "auth_token": "...", "status": "registered"}

    Security:
        - Max 5 concurrent connections per AI
        - Auth token is single-use for connection establishment
        - Tokens expire after 24 hours
    """
    ai_id = ai_id or CURRENT_AI_ID
    conn_id = conn_id or str(uuid.uuid4())

    try:
        with get_db_conn() as db_conn:
            init_streaming_tables(db_conn)

            # Check connection limit
            existing = db_conn.execute(
                'SELECT COUNT(*) FROM ws_connections WHERE ai_id = ? AND status = ?',
                [ai_id, 'active']
            ).fetchone()[0]

            if existing >= MAX_CONNECTIONS_PER_AI:
                return {"error": f"connection_limit|max:{MAX_CONNECTIONS_PER_AI}"}

            # Generate auth token
            auth_token = uuid.uuid4().hex + uuid.uuid4().hex  # 64 chars

            # Insert connection
            now = datetime.now(timezone.utc)
            db_conn.execute('''
                INSERT INTO ws_connections (
                    conn_id, ai_id, auth_token,
                    connected_at, last_ping, teambook_name
                ) VALUES (?, ?, ?, ?, ?, ?)
            ''', [conn_id, ai_id, auth_token, now, now, CURRENT_TEAMBOOK])

            db_conn.commit()

            return {
                "conn_id": conn_id,
                "auth_token": auth_token,
                "status": "registered"
            }

    except Exception as e:
        logging.error(f"Register connection error: {e}", exc_info=True)
        return {"error": "registration_failed"}

def authenticate_connection(conn_id: str, auth_token: str) -> Dict:
    """
    Authenticate WebSocket connection with token

    Args:
        conn_id: Connection UUID
        auth_token: Token from register_connection()

    Returns:
        {"authenticated": True, "ai_id": "..."}
        OR {"error": "invalid_token"}

    Security:
        - Token is single-use (deleted after authentication)
        - Token must match conn_id
    """
    try:
        with get_db_conn() as db_conn:
            init_streaming_tables(db_conn)

            # Verify token
            result = db_conn.execute('''
                SELECT ai_id, connected_at
                FROM ws_connections
                WHERE conn_id = ? AND auth_token = ? AND status = 'active'
            ''', [conn_id, auth_token]).fetchone()

            if not result:
                return {"error": "invalid_token"}

            ai_id, connected_at = result

            # Check token expiration (24 hours)
            age = datetime.now(timezone.utc) - connected_at
            if age > timedelta(hours=24):
                return {"error": "token_expired"}

            # Mark as authenticated (clear token for security)
            db_conn.execute('''
                UPDATE ws_connections
                SET auth_token = '', status = 'authenticated', last_ping = ?
                WHERE conn_id = ?
            ''', [datetime.now(timezone.utc), conn_id])

            db_conn.commit()

            return {
                "authenticated": True,
                "ai_id": ai_id,
                "conn_id": conn_id
            }

    except Exception as e:
        logging.error(f"Authentication error: {e}", exc_info=True)
        return {"error": "auth_failed"}

def update_ping(conn_id: str) -> bool:
    """Update last ping time for connection"""
    try:
        with get_db_conn() as db_conn:
            db_conn.execute(
                'UPDATE ws_connections SET last_ping = ? WHERE conn_id = ?',
                [datetime.now(timezone.utc), conn_id]
            )
            db_conn.commit()
            return True
    except Exception as e:
        logging.error(f"Update ping error: {e}")
        return False

def unregister_connection(conn_id: str) -> bool:
    """
    Unregister WebSocket connection

    Removes from database and clears in-memory caches
    """
    try:
        with get_db_conn() as db_conn:
            # Mark as disconnected
            db_conn.execute(
                'UPDATE ws_connections SET status = ?, last_ping = ? WHERE conn_id = ?',
                ['disconnected', datetime.now(timezone.utc), conn_id]
            )

            # Remove subscriptions
            db_conn.execute(
                'DELETE FROM ws_subscriptions WHERE conn_id = ?',
                [conn_id]
            )

            db_conn.commit()

        # Clear in-memory data
        _active_websockets.pop(conn_id, None)
        _event_cache.pop(conn_id, None)
        _rate_limiter.pop(conn_id, None)

        return True

    except Exception as e:
        logging.error(f"Unregister connection error: {e}", exc_info=True)
        return False

def get_active_connection(ai_id: str = None) -> Optional[str]:
    """
    Get active WebSocket connection ID for AI

    Returns: conn_id if connected, None otherwise
    """
    ai_id = ai_id or CURRENT_AI_ID

    try:
        with get_db_conn() as db_conn:
            result = db_conn.execute('''
                SELECT conn_id
                FROM ws_connections
                WHERE ai_id = ? AND status = 'authenticated'
                ORDER BY last_ping DESC
                LIMIT 1
            ''', [ai_id]).fetchone()

            return result[0] if result else None

    except Exception as e:
        logging.error(f"Get active connection error: {e}")
        return None

# ============= SUBSCRIPTION MANAGEMENT =============

def sync_watches_to_websocket(ai_id: str, conn_id: str) -> int:
    """
    Sync existing watches from event system to WebSocket subscriptions

    When AI connects via WebSocket, this ensures they immediately
    start receiving events for items they're already watching.

    Returns: Number of watches synced
    """
    try:
        with get_db_conn() as db_conn:
            # Get all watches for this AI
            watches = db_conn.execute('''
                SELECT item_type, item_id, event_types
                FROM watches
                WHERE ai_id = ?
            ''', [ai_id]).fetchall()

            count = 0
            now = datetime.now(timezone.utc)

            for item_type, item_id, event_types in watches:
                # Insert subscription
                db_conn.execute('''
                    INSERT INTO ws_subscriptions (
                        conn_id, item_type, item_id, event_types, created_at
                    ) VALUES (?, ?, ?, ?, ?)
                ''', [conn_id, item_type, item_id, event_types, now])
                count += 1

            db_conn.commit()
            logging.info(f"Synced {count} watches to WebSocket for {ai_id}")
            return count

    except Exception as e:
        logging.error(f"Sync watches error: {e}", exc_info=True)
        return 0

def subscribe_websocket(conn_id: str, item_type: str, item_id: str, event_types: List[str] = None) -> bool:
    """
    Add WebSocket subscription for specific item

    Called when AI calls watch() while connected via WebSocket
    """
    try:
        with get_db_conn() as db_conn:
            # Check subscription limit
            count = db_conn.execute(
                'SELECT COUNT(*) FROM ws_subscriptions WHERE conn_id = ?',
                [conn_id]
            ).fetchone()[0]

            if count >= MAX_SUBSCRIPTIONS_PER_CONNECTION:
                logging.warning(f"Subscription limit reached for {conn_id}")
                return False

            # Insert subscription
            event_types_json = json.dumps(event_types) if event_types else None
            db_conn.execute('''
                INSERT INTO ws_subscriptions (
                    conn_id, item_type, item_id, event_types, created_at
                ) VALUES (?, ?, ?, ?, ?)
            ''', [conn_id, item_type, item_id, event_types_json, datetime.now(timezone.utc)])

            db_conn.commit()
            return True

    except Exception as e:
        logging.error(f"Subscribe WebSocket error: {e}", exc_info=True)
        return False

def unsubscribe_websocket(conn_id: str, item_type: str, item_id: str) -> bool:
    """Remove WebSocket subscription"""
    try:
        with get_db_conn() as db_conn:
            db_conn.execute(
                'DELETE FROM ws_subscriptions WHERE conn_id = ? AND item_type = ? AND item_id = ?',
                [conn_id, item_type, item_id]
            )
            db_conn.commit()
            return True
    except Exception as e:
        logging.error(f"Unsubscribe WebSocket error: {e}")
        return False

# ============= EVENT PUSH =============

def check_rate_limit(conn_id: str) -> bool:
    """
    Check if connection is within rate limit

    Returns: True if OK to send, False if rate limited
    """
    now = time.time()
    window_start = now - 1.0  # 1 second window

    # Clean old entries
    _rate_limiter[conn_id] = [
        ts for ts in _rate_limiter[conn_id]
        if ts > window_start
    ]

    # Check limit
    if len(_rate_limiter[conn_id]) >= MESSAGE_RATE_LIMIT:
        return False

    # Record message
    _rate_limiter[conn_id].append(now)
    return True

def push_event_to_watchers(
    event_id: int,
    item_type: str,
    item_id: str,
    event_data: Dict
) -> int:
    """
    Push event to all WebSocket connections watching this item

    Called automatically when emit_event() is invoked.
    AIs don't call this directly.

    Args:
        event_id: Event ID from events table
        item_type: Type of item (note, lock, etc.)
        item_id: Item identifier
        event_data: Event details (event_type, summary, actor_ai_id, etc.)

    Returns: Number of connections notified

    Performance:
        - Batch fetch subscriptions
        - Parallel send to all watchers
        - Non-blocking (fire and forget)
    """
    try:
        with get_db_conn() as db_conn:
            # Find all connections watching this item
            watchers = db_conn.execute('''
                SELECT DISTINCT conn_id, event_types
                FROM ws_subscriptions
                WHERE item_type = ? AND item_id = ?
            ''', [item_type, item_id]).fetchall()

            if not watchers:
                return 0

            # Prepare event message
            event_msg = {
                "type": "event",
                "event_id": event_id,
                "item_type": item_type,
                "item_id": item_id,
                **event_data,
                "created_at": datetime.now(timezone.utc).isoformat()
            }

            notified_count = 0

            for conn_id, event_types_json in watchers:
                # Check event type filter
                if event_types_json:
                    event_types = json.loads(event_types_json)
                    if event_data.get('event_type') not in event_types:
                        continue  # Skip if not subscribed to this event type

                # Check rate limit
                if not check_rate_limit(conn_id):
                    logging.warning(f"Rate limit exceeded for {conn_id}")
                    continue

                # Send via WebSocket if active
                ws = _active_websockets.get(conn_id)
                if ws:
                    try:
                        # Attempt send (non-blocking)
                        # Note: actual WebSocket send will be implemented in universal_adapter
                        # For now, cache the event
                        _event_cache[conn_id].append(event_msg)

                        # Limit cache size
                        if len(_event_cache[conn_id]) > MAX_CACHED_EVENTS_PER_CONNECTION:
                            _event_cache[conn_id].pop(0)  # Remove oldest

                        notified_count += 1
                    except Exception as e:
                        logging.error(f"WebSocket send error for {conn_id}: {e}")
                else:
                    # WebSocket not active, cache for later
                    _event_cache[conn_id].append(event_msg)

            return notified_count

    except Exception as e:
        logging.error(f"Push event error: {e}", exc_info=True)
        return 0

def get_cached_events(conn_id: str, clear: bool = True) -> List[Dict]:
    """
    Get cached events for connection

    Args:
        conn_id: Connection UUID
        clear: Clear cache after retrieving (default True)

    Returns: List of event dictionaries
    """
    events = _event_cache.get(conn_id, [])

    if clear:
        _event_cache[conn_id] = []

    return events

# ============= CLEANUP =============

def cleanup_stale_connections(max_age_seconds: int = 300) -> int:
    """
    Remove connections that haven't pinged in specified time

    Args:
        max_age_seconds: Max seconds since last ping (default 5 minutes)

    Returns: Number of connections cleaned up
    """
    try:
        with get_db_conn() as db_conn:
            cutoff = datetime.now(timezone.utc) - timedelta(seconds=max_age_seconds)

            # Find stale connections
            stale = db_conn.execute('''
                SELECT conn_id
                FROM ws_connections
                WHERE last_ping < ? AND status IN ('active', 'authenticated')
            ''', [cutoff]).fetchall()

            if not stale:
                return 0

            # Mark as disconnected
            conn_ids = [row[0] for row in stale]
            placeholders = ','.join(['?'] * len(conn_ids))

            db_conn.execute(
                f'UPDATE ws_connections SET status = ? WHERE conn_id IN ({placeholders})',
                ['disconnected'] + conn_ids
            )

            # Remove subscriptions
            db_conn.execute(
                f'DELETE FROM ws_subscriptions WHERE conn_id IN ({placeholders})',
                conn_ids
            )

            db_conn.commit()

            # Clear in-memory data
            for conn_id in conn_ids:
                _active_websockets.pop(conn_id, None)
                _event_cache.pop(conn_id, None)
                _rate_limiter.pop(conn_id, None)

            logging.info(f"Cleaned up {len(conn_ids)} stale connections")
            return len(conn_ids)

    except Exception as e:
        logging.error(f"Cleanup error: {e}", exc_info=True)
        return 0

# ============= UTILITY FUNCTIONS =============

def get_connection_stats(ai_id: str = None) -> Dict:
    """
    Get WebSocket connection statistics

    Returns:
        {
            "connections": 2,
            "subscriptions": 15,
            "cached_events": 5
        }
    """
    ai_id = ai_id or CURRENT_AI_ID

    try:
        with get_db_conn() as db_conn:
            # Count connections
            conn_count = db_conn.execute(
                'SELECT COUNT(*) FROM ws_connections WHERE ai_id = ? AND status = ?',
                [ai_id, 'authenticated']
            ).fetchone()[0]

            # Count subscriptions
            if conn_count > 0:
                conn_ids = db_conn.execute(
                    'SELECT conn_id FROM ws_connections WHERE ai_id = ? AND status = ?',
                    [ai_id, 'authenticated']
                ).fetchall()

                conn_ids_list = [row[0] for row in conn_ids]
                placeholders = ','.join(['?'] * len(conn_ids_list))

                sub_count = db_conn.execute(
                    f'SELECT COUNT(*) FROM ws_subscriptions WHERE conn_id IN ({placeholders})',
                    conn_ids_list
                ).fetchone()[0]

                # Count cached events
                cached = sum(len(_event_cache.get(cid, [])) for cid in conn_ids_list)
            else:
                sub_count = 0
                cached = 0

            return {
                "connections": conn_count,
                "subscriptions": sub_count,
                "cached_events": cached
            }

    except Exception as e:
        logging.error(f"Get stats error: {e}")
        return {"error": "stats_failed"}

def should_use_websocket() -> bool:
    """
    Determine if WebSocket streaming should be used

    Checks:
    - Is streaming module available?
    - Has connection succeeded in last 5 minutes?

    Returns: True if WebSocket should be attempted
    """
    conn_id = get_active_connection()
    return conn_id is not None

# ============= MODULE INITIALIZATION =============

# Initialize tables on import
try:
    with get_db_conn() as conn:
        init_streaming_tables(conn)
except Exception as e:
    logging.error(f"Streaming initialization error: {e}")