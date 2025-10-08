#!/usr/bin/env python3
"""
TEAMBOOK V3 - PRESENCE TRACKING
================================
Activity-based presence system for AI coordination.

Design goals:
1. Zero-overhead - updates on any teambook operation (passive)
2. Rich status - online/away with custom status messages
3. Last-seen tracking - know when AIs were last active
4. Multi-teambook aware - presence per teambook

Built by AIs, for AIs.
"""

import time
from datetime import datetime, timedelta, timezone
from typing import Dict, List, Optional, Tuple
from dataclasses import dataclass
from enum import Enum

from teambook_storage import get_db_conn
from teambook_shared import CURRENT_AI_ID, CURRENT_TEAMBOOK


class PresenceStatus(Enum):
    """AI presence status"""
    ONLINE = "online"      # Active within last 2 minutes
    AWAY = "away"          # Active within last 15 minutes
    OFFLINE = "offline"    # No activity in 15+ minutes


@dataclass
class AIPresence:
    """Presence information for an AI"""
    ai_id: str
    status: PresenceStatus
    last_seen: datetime
    status_message: Optional[str] = None
    teambook_name: Optional[str] = None

    def minutes_ago(self) -> int:
        """Calculate minutes since last seen"""
        delta = datetime.now(timezone.utc) - self.last_seen
        return int(delta.total_seconds() / 60)

    def status_indicator(self) -> str:
        """Get emoji/symbol for status"""
        return {
            PresenceStatus.ONLINE: "🟢",
            PresenceStatus.AWAY: "🟡",
            PresenceStatus.OFFLINE: "🔴"
        }[self.status]


# ============= DATABASE SCHEMA =============

def init_presence_tables(conn):
    """Initialize presence tracking tables"""

    conn.execute('''
        CREATE TABLE IF NOT EXISTS ai_presence (
            ai_id VARCHAR(100) PRIMARY KEY,
            teambook_name VARCHAR(50),
            last_seen TIMESTAMPTZ NOT NULL,
            last_operation VARCHAR(50),
            status_message VARCHAR(200),
            updated TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
    ''')

    # Index for efficient queries
    conn.execute('''
        CREATE INDEX IF NOT EXISTS idx_presence_lastseen
        ON ai_presence(teambook_name, last_seen DESC)
    ''')

    conn.execute('''
        CREATE INDEX IF NOT EXISTS idx_presence_teambook
        ON ai_presence(teambook_name, last_seen DESC)
    ''')


# ============= PRESENCE UPDATES =============

def update_presence(
    ai_id: str = None,
    operation: str = None,
    status_message: str = None,
    teambook_name: str = None
):
    """
    Update AI presence - called automatically on any teambook operation.

    Parameters:
    - ai_id: AI identifier (defaults to CURRENT_AI_ID)
    - operation: What operation triggered the update (optional, for debugging)
    - status_message: Custom status message (optional, e.g., "Working on docs")
    - teambook_name: Which teambook (defaults to CURRENT_TEAMBOOK)
    """
    ai_id = ai_id or CURRENT_AI_ID
    teambook_name = teambook_name or CURRENT_TEAMBOOK

    if not ai_id:
        return  # Can't track presence without AI ID

    try:
        with get_db_conn() as conn:
            # Ensure table exists
            init_presence_tables(conn)

            now = datetime.now(timezone.utc)

            # Upsert presence record
            conn.execute('''
                INSERT INTO ai_presence (ai_id, teambook_name, last_seen, last_operation, status_message, updated)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT (ai_id) DO UPDATE SET
                    teambook_name = EXCLUDED.teambook_name,
                    last_seen = EXCLUDED.last_seen,
                    last_operation = EXCLUDED.last_operation,
                    status_message = CASE
                        WHEN EXCLUDED.status_message IS NOT NULL
                        THEN EXCLUDED.status_message
                        ELSE ai_presence.status_message
                    END,
                    updated = EXCLUDED.updated
            ''', [ai_id, teambook_name, now, operation, status_message, now])

    except Exception as e:
        # Presence tracking is non-critical - don't break operations if it fails
        import logging
        logging.debug(f"Presence update failed (non-critical): {e}")


def set_status(
    status_message: str,
    ai_id: str = None
):
    """
    Set custom status message for this AI.

    Examples:
    - "Working on GitHub cleanup"
    - "Reviewing code"
    - "Away - back in 10 min"
    """
    update_presence(
        ai_id=ai_id,
        operation="set_status",
        status_message=status_message
    )


def clear_status(ai_id: str = None):
    """Clear custom status message"""
    update_presence(
        ai_id=ai_id,
        operation="clear_status",
        status_message=None
    )


# ============= PRESENCE QUERIES =============

def get_presence(ai_id: str, teambook_name: str = None) -> Optional[AIPresence]:
    """
    Get presence info for a specific AI.

    Returns None if AI has never been seen.
    """
    teambook_name = teambook_name or CURRENT_TEAMBOOK

    try:
        with get_db_conn() as conn:
            init_presence_tables(conn)

            result = conn.execute('''
                SELECT ai_id, last_seen, status_message, teambook_name
                FROM ai_presence
                WHERE ai_id = ?
            ''', [ai_id]).fetchone()

            if not result:
                return None

            last_seen = result[1]
            if isinstance(last_seen, str):
                last_seen = datetime.fromisoformat(last_seen)

            # Calculate status based on last_seen
            minutes_ago = (datetime.now(timezone.utc) - last_seen).total_seconds() / 60

            if minutes_ago < 2:
                status = PresenceStatus.ONLINE
            elif minutes_ago < 15:
                status = PresenceStatus.AWAY
            else:
                status = PresenceStatus.OFFLINE

            return AIPresence(
                ai_id=result[0],
                status=status,
                last_seen=last_seen,
                status_message=result[2],
                teambook_name=result[3]
            )

    except Exception as e:
        import logging
        logging.debug(f"Get presence failed: {e}")
        return None


def who_is_here(
    minutes: int = 15,
    teambook_name: str = None
) -> List[AIPresence]:
    """
    Get all AIs active within the last N minutes in this teambook.

    Parameters:
    - minutes: Consider AIs active within this many minutes (default: 15)
    - teambook_name: Filter by teambook (default: current teambook)

    Returns list sorted by most recently active first.
    """
    teambook_name = teambook_name or CURRENT_TEAMBOOK

    try:
        with get_db_conn() as conn:
            init_presence_tables(conn)

            cutoff = datetime.now(timezone.utc) - timedelta(minutes=minutes)

            query = '''
                SELECT ai_id, last_seen, status_message, teambook_name
                FROM ai_presence
                WHERE last_seen >= ?
            '''
            params = [cutoff]

            if teambook_name:
                query += ' AND teambook_name = ?'
                params.append(teambook_name)

            query += ' ORDER BY last_seen DESC'

            results = conn.execute(query, params).fetchall()

            presences = []
            for row in results:
                last_seen = row[1]
                if isinstance(last_seen, str):
                    last_seen = datetime.fromisoformat(last_seen)

                minutes_ago = (datetime.now(timezone.utc) - last_seen).total_seconds() / 60

                if minutes_ago < 2:
                    status = PresenceStatus.ONLINE
                elif minutes_ago < 15:
                    status = PresenceStatus.AWAY
                else:
                    status = PresenceStatus.OFFLINE

                presences.append(AIPresence(
                    ai_id=row[0],
                    status=status,
                    last_seen=last_seen,
                    status_message=row[2],
                    teambook_name=row[3]
                ))

            return presences

    except Exception as e:
        import logging
        logging.debug(f"Who is here query failed: {e}")
        return []


def get_all_presence(
    teambook_name: str = None,
    include_offline: bool = False
) -> List[AIPresence]:
    """
    Get presence for ALL AIs ever seen in this teambook.

    Parameters:
    - teambook_name: Filter by teambook (default: current teambook)
    - include_offline: Include AIs that are offline (default: False)

    Returns list sorted by most recently active first.
    """
    teambook_name = teambook_name or CURRENT_TEAMBOOK

    try:
        with get_db_conn() as conn:
            init_presence_tables(conn)

            query = 'SELECT ai_id, last_seen, status_message, teambook_name FROM ai_presence'
            params = []

            if teambook_name:
                query += ' WHERE teambook_name = ?'
                params.append(teambook_name)

            query += ' ORDER BY last_seen DESC'

            results = conn.execute(query, params).fetchall()

            presences = []
            for row in results:
                last_seen = row[1]
                if isinstance(last_seen, str):
                    last_seen = datetime.fromisoformat(last_seen)

                minutes_ago = (datetime.now(timezone.utc) - last_seen).total_seconds() / 60

                if minutes_ago < 2:
                    status = PresenceStatus.ONLINE
                elif minutes_ago < 15:
                    status = PresenceStatus.AWAY
                else:
                    status = PresenceStatus.OFFLINE

                if not include_offline and status == PresenceStatus.OFFLINE:
                    continue

                presences.append(AIPresence(
                    ai_id=row[0],
                    status=status,
                    last_seen=last_seen,
                    status_message=row[2],
                    teambook_name=row[3]
                ))

            return presences

    except Exception as e:
        import logging
        logging.debug(f"Get all presence failed: {e}")
        return []


# ============= CLEANUP =============

def cleanup_old_presence(days: int = 30):
    """
    Remove presence records older than N days.

    This prevents unbounded growth of the presence table.
    Called periodically (e.g., daily) via a maintenance task.
    """
    try:
        with get_db_conn() as conn:
            cutoff = datetime.now(timezone.utc) - timedelta(days=days)

            result = conn.execute('''
                DELETE FROM ai_presence
                WHERE last_seen < ?
            ''', [cutoff])

            deleted = result.fetchall() if hasattr(result, 'fetchall') else 0

            import logging
            if deleted:
                logging.info(f"Cleaned up {deleted} old presence records")

    except Exception as e:
        import logging
        logging.debug(f"Presence cleanup failed (non-critical): {e}")


# ============= AUTOMATIC PRESENCE TRACKING =============

def track_operation(operation: str):
    """
    Decorator to automatically track presence on function calls.

    Usage:
    @track_operation("broadcast")
    def broadcast(channel, content):
        ...
    """
    def decorator(func):
        def wrapper(*args, **kwargs):
            # Update presence before operation
            update_presence(operation=operation)
            # Execute operation
            return func(*args, **kwargs)
        return wrapper
    return decorator
