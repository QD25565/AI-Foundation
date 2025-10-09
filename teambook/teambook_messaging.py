#!/usr/bin/env python3
"""
TEAMBOOK MESSAGING v1.0.0 - SECURE MULTI-AGENT COMMUNICATION
==============================================================
High-performance, secure pub/sub messaging for AI collaboration.
Zero-trust design, no JSON responses, forgiving input handling.

Security Features:
- Rate limiting per AI (prevents spam/DoS)
- Message size limits (prevents memory exhaustion)
- Channel name validation (prevents injection)
- Expiring messages (prevents unbounded growth)
- SQL injection prevention (parameterized queries)
- Input sanitization (forgiving but secure)
"""

import re
import time
from datetime import datetime, timedelta, timezone
from typing import Dict, List, Optional, Tuple, Set
from collections import defaultdict
import logging

from teambook_shared import (
    CURRENT_AI_ID, CURRENT_TEAMBOOK, OUTPUT_FORMAT,
    pipe_escape, format_time_compact, clean_text,
    build_security_envelope, get_registered_human_identity
)

from teambook_storage import get_db_conn, log_operation_to_db
from teambook_pubsub import publish_broadcast, publish_direct_message

# ============= SECURITY LIMITS =============

MAX_MESSAGE_LENGTH = 5000  # Increased for V3 - matches enhanced content needs
MAX_MESSAGE_SUMMARY_LENGTH = 400  # V3: Summary field limit
MAX_CHANNEL_NAME_LENGTH = 50
MAX_CHANNELS_PER_AI = 20  # Prevent resource exhaustion
MAX_MESSAGES_PER_MINUTE = 100  # Rate limit per AI
MESSAGE_RETENTION_HOURS = 24  # Auto-cleanup old messages
MAX_BROADCAST_SIZE = 50  # Max recipients per broadcast

# Rate limiting state (in-memory for performance)
_rate_limiter = defaultdict(list)  # ai_id -> [timestamps]
_subscriptions = defaultdict(set)  # ai_id -> {channels}

# ============= INPUT VALIDATION =============

def sanitize_channel(channel: str, allow_wildcards: bool = False) -> Optional[str]:
    """
    Sanitize channel name - SECURITY CRITICAL

    Returns None if invalid, sanitized string if valid.
    Forgiving: converts to lowercase, strips whitespace.
    """
    if not channel:
        return None

    # Convert to string and normalize
    channel = str(channel).strip().lower()

    # Length check
    if len(channel) > MAX_CHANNEL_NAME_LENGTH:
        return None

    # Character whitelist (alphanumeric, dash, underscore, optional wildcards)
    if allow_wildcards:
        pattern = r'^[a-z0-9_\-\*]+$'
    else:
        pattern = r'^[a-z0-9_\-]+$'

    if not re.match(pattern, channel):
        return None

    return channel

def sanitize_ai_id(ai_id: str) -> Optional[str]:
    """
    Sanitize AI identifier - SECURITY CRITICAL

    Returns None if invalid, sanitized string if valid.
    """
    if not ai_id:
        return None

    ai_id = str(ai_id).strip()

    # Length check
    if len(ai_id) > 100:
        return None

    # Character whitelist
    if not re.match(r'^[A-Za-z0-9_\-]+$', ai_id):
        return None

    return ai_id

def sanitize_message(content: str) -> Tuple[str, bool]:
    """
    Sanitize message content.

    Returns (sanitized_content, was_truncated)
    Forgiving: allows most content, just limits length.
    """
    if not content:
        return "", False

    content = clean_text(str(content))
    truncated = False

    if len(content) > MAX_MESSAGE_LENGTH:
        content = content[:MAX_MESSAGE_LENGTH]
        truncated = True

    return content, truncated

def check_rate_limit(ai_id: str) -> Tuple[bool, int]:
    """
    Check if AI is within rate limits.

    Returns (allowed, remaining_quota)
    """
    now = time.time()
    minute_ago = now - 60

    # Clean old timestamps
    _rate_limiter[ai_id] = [t for t in _rate_limiter[ai_id] if t > minute_ago]

    current_count = len(_rate_limiter[ai_id])
    remaining = MAX_MESSAGES_PER_MINUTE - current_count

    if current_count >= MAX_MESSAGES_PER_MINUTE:
        return False, 0

    _rate_limiter[ai_id].append(now)
    return True, remaining - 1

# ============= DATABASE INITIALIZATION =============

def init_messaging_tables(conn):
    """Initialize messaging tables with proper indexes"""

    # Create sequence for auto-increment
    try:
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_messages')
    except Exception:
        pass  # Sequence might already exist

    conn.execute('''
        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_messages'),
            channel VARCHAR(50) NOT NULL,
            from_ai VARCHAR(100) NOT NULL,
            to_ai VARCHAR(100),
            content TEXT NOT NULL,
            created TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            read BOOLEAN DEFAULT FALSE,
            expires_at TIMESTAMPTZ NOT NULL,
            summary TEXT,
            reply_to INTEGER,
            teambook_name VARCHAR(50),
            signature VARCHAR(128),
            security_envelope TEXT,
            identity_hint TEXT
        )
    ''')

    # Indexes for performance
    conn.execute('CREATE INDEX IF NOT EXISTS idx_msg_channel ON messages(channel, created DESC)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_msg_to_ai ON messages(to_ai, read, created DESC)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_msg_expires ON messages(expires_at)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_msg_from_ai ON messages(from_ai, created DESC)')

    columns = {col[1] for col in conn.execute("PRAGMA table_info(messages)").fetchall()}
    if 'summary' not in columns:
        conn.execute("ALTER TABLE messages ADD COLUMN summary TEXT")
    if 'reply_to' not in columns:
        conn.execute("ALTER TABLE messages ADD COLUMN reply_to INTEGER")
    if 'teambook_name' not in columns:
        conn.execute("ALTER TABLE messages ADD COLUMN teambook_name VARCHAR(50)")
    if 'signature' not in columns:
        conn.execute("ALTER TABLE messages ADD COLUMN signature VARCHAR(128)")
    if 'security_envelope' not in columns:
        conn.execute("ALTER TABLE messages ADD COLUMN security_envelope TEXT")
    if 'identity_hint' not in columns:
        conn.execute("ALTER TABLE messages ADD COLUMN identity_hint TEXT")

    conn.commit()

def cleanup_expired_messages(conn):
    """Remove expired messages - called periodically"""
    try:
        deleted = conn.execute(
            'DELETE FROM messages WHERE expires_at < ?',
            [datetime.now(timezone.utc)]
        ).rowcount

        if deleted > 0:
            conn.commit()
            logging.info(f"Cleaned up {deleted} expired messages")
    except Exception as e:
        logging.error(f"Message cleanup error: {e}")


def _prepare_message_security(channel: str, content: str, to_ai: Optional[str], expires_at: datetime) -> Tuple[Optional[str], Optional[str], Optional[str]]:
    """Generate signature, envelope, and identity hint for a message payload."""

    payload = {
        'ai_id': CURRENT_AI_ID,
        'channel': channel,
        'recipient': to_ai,
        'content_hash': hashlib.sha3_256((content or '').encode('utf-8')).hexdigest(),
        'expires_at': expires_at.isoformat(),
        'teambook': CURRENT_TEAMBOOK,
    }
    envelope = build_security_envelope(payload, 'teambook.messaging.dispatch')
    signature = envelope.get('signature') if envelope else None
    envelope_json = json.dumps(envelope, sort_keys=True) if envelope else None
    identity_hint = get_registered_human_identity(CURRENT_AI_ID)
    identity_json = json.dumps(identity_hint, sort_keys=True) if identity_hint else None
    return signature, envelope_json, identity_json

# ============= CORE MESSAGING FUNCTIONS =============

def broadcast(content: str = None, channel: str = "general", ttl_hours: int = 24, **kwargs) -> Dict:
    """
    Broadcast message to a channel.

    Security: Rate limited, input sanitized, size limited.
    Forgiving: Handles malformed input gracefully.
    """
    try:
        # Rate limiting
        allowed, remaining = check_rate_limit(CURRENT_AI_ID)
        if not allowed:
            return "!rate_limit:wait_60s"

        # Sanitize inputs
        channel = sanitize_channel(kwargs.get('channel', channel))
        if not channel:
            return "!invalid_channel:alphanumeric_only"

        content, truncated = sanitize_message(kwargs.get('content', content))
        if not content:
            return "!empty_message"

        # TTL validation
        ttl_hours = int(kwargs.get('ttl_hours', ttl_hours or 24))
        if ttl_hours < 1 or ttl_hours > 168:  # Max 7 days
            ttl_hours = 24

        expires_at = datetime.now(timezone.utc) + timedelta(hours=ttl_hours)
        signature_value, envelope_json, identity_json = _prepare_message_security(channel, content, None, expires_at)

        with get_db_conn() as conn:
            init_messaging_tables(conn)

            cursor = conn.execute('''
                INSERT INTO messages (
                    channel, from_ai, content, created, expires_at,
                    teambook_name, signature, security_envelope, identity_hint
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                RETURNING id
            ''', [
                channel,
                CURRENT_AI_ID,
                content,
                datetime.now(timezone.utc),
                expires_at,
                CURRENT_TEAMBOOK,
                signature_value,
                envelope_json,
                identity_json,
            ])

            msg_id = cursor.fetchone()[0]

            # Periodic cleanup (10% chance on each broadcast)
            import random
            if random.random() < 0.1:
                cleanup_expired_messages(conn)

            # CRITICAL: Explicit commit BEFORE Redis publish to avoid race condition
            # This ensures DB write is visible to other connections before event fires
            conn.commit()

        log_operation_to_db('broadcast')

        # Publish Redis event for real-time notifications
        # (DB is committed above, so message is guaranteed to be readable)
        try:
            publish_broadcast(channel, content)
        except Exception as e:
            logging.warning(f"Failed to publish broadcast event: {e}")

        # Pure pipe format (token optimized - no dict wrapper!)
        result = f"msg:{msg_id}|{channel}|{format_time_compact(datetime.now(timezone.utc))}"
        if truncated:
            result += "|truncated"
        if remaining < 10:
            result += f"|quota:{remaining}"

        # Add standby hint for collaboration
        result += "\n\n💡 Waiting for responses? Use: teambook_standby_mode"

        return result  # Direct string, no dict!

    except Exception as e:
        logging.error(f"Broadcast error: {e}")
        return "!broadcast_failed"  # Error prefix with !

def direct_message(to_ai: str = None, content: str = None, ttl_hours: int = 24, **kwargs) -> Dict:
    """
    Send direct message to specific AI.

    Security: Rate limited, input validated, recipient verified.
    """
    try:
        # Rate limiting
        allowed, remaining = check_rate_limit(CURRENT_AI_ID)
        if not allowed:
            return "!rate_limit:wait_60s"

        # Sanitize inputs
        to_ai = sanitize_ai_id(kwargs.get('to_ai', to_ai))
        if not to_ai:
            return "!invalid_recipient"

        if to_ai == CURRENT_AI_ID:
            return "!cannot_dm_self"

        # Check if recipient AI has ever been active (UX improvement)
        # This warns users if they're messaging an AI that doesn't exist
        with get_db_conn() as conn:
            ai_exists = conn.execute(
                'SELECT COUNT(*) FROM messages WHERE from_ai = ? OR to_ai = ? LIMIT 1',
                [to_ai, to_ai]
            ).fetchone()[0]

        if ai_exists == 0:
            # AI has never sent or received messages - likely doesn't exist
            # Still send the message but add a warning flag
            recipient_exists = False
        else:
            recipient_exists = True

        content, truncated = sanitize_message(kwargs.get('content', content))
        if not content:
            return "!empty_message"

        ttl_hours = int(kwargs.get('ttl_hours', ttl_hours or 24))
        if ttl_hours < 1 or ttl_hours > 168:
            ttl_hours = 24

        expires_at = datetime.now(timezone.utc) + timedelta(hours=ttl_hours)
        signature_value, envelope_json, identity_json = _prepare_message_security('_dm', content, to_ai, expires_at)

        with get_db_conn() as conn:
            init_messaging_tables(conn)

            cursor = conn.execute('''
                INSERT INTO messages (
                    channel, from_ai, to_ai, content, created, expires_at,
                    teambook_name, signature, security_envelope, identity_hint
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                RETURNING id
            ''', [
                '_dm',
                CURRENT_AI_ID,
                to_ai,
                content,
                datetime.now(timezone.utc),
                expires_at,
                CURRENT_TEAMBOOK,
                signature_value,
                envelope_json,
                identity_json,
            ])

            msg_id = cursor.fetchone()[0]

        log_operation_to_db('direct_message')

        # Publish Redis event for real-time notifications
        try:
            publish_direct_message(to_ai, content)
        except Exception as e:
            logging.warning(f"Failed to publish DM event: {e}")

        # Pure pipe format (token optimized!)
        result = f"dm:{msg_id}|to:{to_ai}|{format_time_compact(datetime.now(timezone.utc))}"
        if truncated:
            result += "|truncated"
        if not recipient_exists:
            result += "|!warn:recipient_unknown"

        # Add standby hint for collaboration
        result += "\n\n💡 Waiting for a response? Use: teambook_standby_mode"

        return result

    except Exception as e:
        logging.error(f"DM error: {e}")
        return "!dm_failed"

def subscribe(channel: str = None, **kwargs) -> Dict:
    """
    Subscribe to a channel for notifications.

    Note: Subscriptions are in-memory (not persisted).
    """
    try:
        channel = sanitize_channel(kwargs.get('channel', channel), allow_wildcards=True)
        if not channel:
            return "!invalid_channel"

        # Check subscription limit
        if len(_subscriptions[CURRENT_AI_ID]) >= MAX_CHANNELS_PER_AI:
            return f"!max_channels|limit:{MAX_CHANNELS_PER_AI}"

        _subscriptions[CURRENT_AI_ID].add(channel)

        count = len(_subscriptions[CURRENT_AI_ID])
        # Pure pipe format (token optimized!)
        return f"{channel}|total:{count}"

    except Exception as e:
        logging.error(f"Subscribe error: {e}")
        return "!subscribe_failed"

def unsubscribe(channel: str = None, **kwargs) -> Dict:
    """Unsubscribe from a channel"""
    try:
        channel = sanitize_channel(kwargs.get('channel', channel), allow_wildcards=True)
        if not channel:
            return "!invalid_channel"

        _subscriptions[CURRENT_AI_ID].discard(channel)

        # Pure pipe format (token optimized!)
        return channel

    except Exception as e:
        logging.error(f"Unsubscribe error: {e}")
        return "!unsubscribe_failed"

def get_subscriptions(**kwargs) -> Dict:
    """List current subscriptions"""
    channels = list(_subscriptions[CURRENT_AI_ID])

    if not channels:
        return ""  # Empty string for no subscriptions (token optimized!)

    # Pure pipe format (token optimized!)
    return '|'.join(channels)

def read_channel(channel: str = None, limit: int = 20, unread_only: bool = False, **kwargs) -> Dict:
    """
    Read messages from a channel.

    Security: Validates channel access, limits result set.
    Performance: Uses indexed queries, batch marking as read.
    """
    try:
        channel = sanitize_channel(kwargs.get('channel', channel))
        if not channel:
            return "!invalid_channel"

        limit = int(kwargs.get('limit', limit or 20))
        if limit < 1 or limit > 100:
            limit = 20

        unread_only = bool(kwargs.get('unread_only', unread_only))

        with get_db_conn() as conn:
            init_messaging_tables(conn)

            # Query with security filters
            if unread_only:
                query = '''
                    SELECT id, from_ai, content, created
                    FROM messages
                    WHERE channel = ? AND expires_at > ? AND read = FALSE
                    ORDER BY created DESC
                    LIMIT ?
                '''
            else:
                query = '''
                    SELECT id, from_ai, content, created
                    FROM messages
                    WHERE channel = ? AND expires_at > ?
                    ORDER BY created DESC
                    LIMIT ?
                '''

            messages = conn.execute(query, [
                channel,
                datetime.now(timezone.utc),
                limit
            ]).fetchall()

            if not messages:
                return ""  # Empty string for no results (token optimized!)

            # Mark as read (batch operation)
            msg_ids = [msg[0] for msg in messages]
            placeholders = ','.join(['?'] * len(msg_ids))
            conn.execute(f'''
                UPDATE messages
                SET read = TRUE
                WHERE id IN ({placeholders})
            ''', msg_ids)

        log_operation_to_db('read_channel')

        # Pure pipe format (token optimized!) - newline separated
        lines = []
        for msg_id, from_ai, content, created in messages:
            # Format: id|from|time|content
            parts = [
                str(msg_id),
                from_ai,
                format_time_compact(created),
                content[:5000]  # Full content (5000 chars - matches DM limit)
            ]
            lines.append('|'.join(pipe_escape(p) for p in parts))
        return '\n'.join(lines)  # Direct string, newline separated!

    except Exception as e:
        logging.error(f"Read channel error: {e}")
        return "!read_failed"

def read_dms(limit: int = 20, unread_only: bool = True, **kwargs) -> Dict:
    """
    Read direct messages sent to this AI - FULL CONTENT (no truncation).

    DMs show complete message content (up to 5000 chars) since they are
    direct 1:1 communication requiring full context.

    Best practice: Keep DMs reasonably sized when possible.

    Security: Only returns messages addressed to current AI.
    """
    try:
        limit = int(kwargs.get('limit', limit or 20))
        if limit < 1 or limit > 100:
            limit = 20

        unread_only = bool(kwargs.get('unread_only', unread_only if unread_only is not None else True))

        with get_db_conn() as conn:
            init_messaging_tables(conn)

            if unread_only:
                query = '''
                    SELECT id, from_ai, content, created
                    FROM messages
                    WHERE to_ai = ? AND expires_at > ? AND read = FALSE
                    ORDER BY created DESC
                    LIMIT ?
                '''
            else:
                query = '''
                    SELECT id, from_ai, content, created
                    FROM messages
                    WHERE to_ai = ? AND expires_at > ?
                    ORDER BY created DESC
                    LIMIT ?
                '''

            messages = conn.execute(query, [
                CURRENT_AI_ID,
                datetime.now(timezone.utc),
                limit
            ]).fetchall()

            if not messages:
                return ""  # Empty string for no DMs (token optimized!)

            # Mark as read
            msg_ids = [msg[0] for msg in messages]
            placeholders = ','.join(['?'] * len(msg_ids))
            conn.execute(f'''
                UPDATE messages
                SET read = TRUE
                WHERE id IN ({placeholders})
            ''', msg_ids)

        log_operation_to_db('read_dms')

        # Pure pipe format - newline separated
        # DMs show FULL content (no truncation) for complete context
        lines = []
        for msg_id, from_ai, content, created in messages:
            parts = [
                f"dm:{msg_id}",
                from_ai,
                format_time_compact(created),
                content  # Full content - no [:100] truncation
            ]
            lines.append('|'.join(pipe_escape(p) for p in parts))
        return '\n'.join(lines)  # Direct string, newline separated!

    except Exception as e:
        logging.error(f"Read DMs error: {e}")
        return "!read_dms_failed"

def message_stats(**kwargs) -> Dict:
    """Get messaging statistics"""
    try:
        with get_db_conn() as conn:
            init_messaging_tables(conn)

            stats = conn.execute('''
                SELECT
                    COUNT(*) as total,
                    COUNT(CASE WHEN to_ai = ? AND read = FALSE THEN 1 END) as unread_dms,
                    COUNT(CASE WHEN from_ai = ? THEN 1 END) as sent_by_me,
                    COUNT(DISTINCT channel) as channels
                FROM messages
                WHERE expires_at > ?
            ''', [CURRENT_AI_ID, CURRENT_AI_ID, datetime.now(timezone.utc)]).fetchone()

            total, unread_dms, sent, channels = stats

        # Rate limit info
        allowed, remaining = check_rate_limit(CURRENT_AI_ID)
        _rate_limiter[CURRENT_AI_ID].pop()  # Don't count this check

        # Pure pipe format (token optimized!)
        parts = [
            f"total:{total}",
            f"unread:{unread_dms}",
            f"sent:{sent}",
            f"channels:{channels}",
            f"quota:{remaining}"
        ]
        return '|'.join(parts)  # Direct string!

    except Exception as e:
        logging.error(f"Stats error: {e}")
        return "!stats_failed"


# ============================================================================
# TEAMBOOK V3 - STRUCTURED DATA API
# ============================================================================
# V3 functions return structured dicts for CLI + MCP dual interface support
# Old functions remain for backward compatibility

# Standard error codes for programmatic checking
ERROR_CODES = {
    "rate_limit": "Rate limit exceeded",
    "invalid_channel": "Channel name invalid or not found",
    "invalid_recipient": "Recipient AI ID invalid",
    "cannot_dm_self": "Cannot send direct message to yourself",
    "empty_message": "Message content is empty",
    "message_too_long": "Message exceeds maximum length",
    "summary_too_long": "Summary exceeds maximum length",
    "recipient_unknown": "Recipient AI has never been active",
    "broadcast_failed": "Failed to broadcast message",
    "dm_failed": "Failed to send direct message"
}

def send_message(
    content: str = None,
    to: str = None,
    channel: str = "general",
    reply_to: int = None,
    summary: str = None,
    ttl_hours: int = 24,
    **kwargs
) -> Dict:
    """
    UNIFIED MESSAGING - Send broadcast or direct message

    Args:
        content: Message content (required, max 5000 chars)
        to: Recipient AI ID for DM, None/null for broadcast
        channel: Channel name (default: "general", ignored for DMs)
        reply_to: Message ID to reply to (creates thread)
        summary: Optional summary (max 400 chars, auto-generated if omitted)
        ttl_hours: Time to live in hours (1-168, default 24)

    Returns:
        Dict with success status and full metadata:
        {
            "success": True/False,
            "msg_id": 123,
            "from": "claude-instance-1",
            "to": "all" or "claude-instance-2",
            "channel": "general" or "_dm",
            "timestamp": "2025-10-02T10:30:00Z",
            "reply_to": 340 or None,
            "summary": "Message summary...",
            "recipients_count": 3,
            "error": "error_code" (if failed),
            "message": "Human-readable error" (if failed),
            "details": {...} (if failed),
            "suggestion": "How to fix" (if failed)
        }

    This replaces both broadcast() and direct_message() with unified interface.
    """
    try:
        # Rate limiting
        allowed, remaining = check_rate_limit(CURRENT_AI_ID)
        if not allowed:
            return {
                "success": False,
                "error": "rate_limit",
                "message": "Rate limit exceeded (100 messages/minute)",
                "details": {
                    "limit": MAX_MESSAGES_PER_MINUTE,
                    "window": "60 seconds"
                },
                "suggestion": "Wait 60 seconds before sending more messages"
            }

        # Extract from kwargs for flexibility
        content = kwargs.get('content', content)
        to = kwargs.get('to', to)
        channel = kwargs.get('channel', channel)
        reply_to = kwargs.get('reply_to', reply_to)
        summary = kwargs.get('summary', summary)
        ttl_hours = kwargs.get('ttl_hours', ttl_hours)

        # Determine if this is a DM or broadcast
        is_dm = to is not None and to != "all"

        # Validate recipient for DM
        if is_dm:
            to = sanitize_ai_id(to)
            if not to:
                return {
                    "success": False,
                    "error": "invalid_recipient",
                    "message": "Recipient AI ID is invalid",
                    "details": {
                        "provided": kwargs.get('to', '(empty)')
                    },
                    "suggestion": "Provide a valid AI ID (e.g., 'claude-instance-2')"
                }

            if to == CURRENT_AI_ID:
                return {
                    "success": False,
                    "error": "cannot_dm_self",
                    "message": "Cannot send direct message to yourself",
                    "suggestion": "Omit 'to' parameter to broadcast to all AIs"
                }

        # Validate channel for broadcast
        if not is_dm:
            channel = sanitize_channel(channel)
            if not channel:
                return {
                    "success": False,
                    "error": "invalid_channel",
                    "message": "Channel name is invalid",
                    "details": {
                        "provided": kwargs.get('channel', '(empty)'),
                        "allowed": "alphanumeric, dash, underscore only"
                    },
                    "suggestion": "Use channel name like 'general' or 'project-updates'"
                }

        # Sanitize and validate content
        content, truncated = sanitize_message(content)
        if not content:
            return {
                "success": False,
                "error": "empty_message",
                "message": "Message content is empty or invalid",
                "suggestion": "Provide content parameter with your message text"
            }

        # Handle summary - auto-generate if not provided
        if not summary:
            summary = auto_summarize_message(content)
        else:
            # Validate summary length
            summary = clean_text(str(summary))
            if len(summary) > MAX_MESSAGE_SUMMARY_LENGTH:
                summary = summary[:MAX_MESSAGE_SUMMARY_LENGTH]

        # TTL validation
        ttl_hours = int(ttl_hours or 24)
        if ttl_hours < 1 or ttl_hours > 168:
            ttl_hours = 24

        expires_at = datetime.now(timezone.utc) + timedelta(hours=ttl_hours)

        # Check if recipient exists (for DMs)
        recipient_exists = True
        if is_dm:
            with get_db_conn() as conn:
                ai_exists = conn.execute(
                    'SELECT COUNT(*) FROM messages WHERE from_ai = ? OR to_ai = ? LIMIT 1',
                    [to, to]
                ).fetchone()[0]
                recipient_exists = ai_exists > 0

        # Store message in database
        with get_db_conn() as conn:
            init_messaging_tables(conn)

            signature_value, envelope_json, identity_json = _prepare_message_security(
                '_dm' if is_dm else channel,
                content,
                to if is_dm else None,
                expires_at,
            )

            cursor = conn.execute('''
                INSERT INTO messages (
                    channel, from_ai, to_ai, content, summary, reply_to, created, expires_at,
                    teambook_name, signature, security_envelope, identity_hint
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                RETURNING id
            ''', [
                '_dm' if is_dm else channel,
                CURRENT_AI_ID,
                to if is_dm else None,
                content,
                summary,
                reply_to,
                datetime.now(timezone.utc),
                expires_at,
                CURRENT_TEAMBOOK,
                signature_value,
                envelope_json,
                identity_json,
            ])

            msg_id = cursor.fetchone()[0]

            # Periodic cleanup
            import random
            if random.random() < 0.1:
                cleanup_expired_messages(conn)

        # Count active recipients
        if is_dm:
            recipients_count = 1
        else:
            with get_db_conn() as conn:
                # Count unique AIs who've been active in last 24h
                cutoff = datetime.now(timezone.utc) - timedelta(hours=24)
                recipients_count = conn.execute('''
                    SELECT COUNT(DISTINCT from_ai)
                    FROM messages
                    WHERE created > ? AND from_ai != ?
                ''', [cutoff, CURRENT_AI_ID]).fetchone()[0]

        log_operation_to_db('send_message_v3')

        # Return structured data
        result = {
            "success": True,
            "msg_id": msg_id,
            "from": CURRENT_AI_ID,
            "to": to if is_dm else "all",
            "channel": "_dm" if is_dm else channel,
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "reply_to": reply_to,
            "summary": summary,
            "recipients_count": recipients_count
        }

        # Add warnings
        if truncated:
            result["warning"] = "message_truncated"
            result["warning_message"] = f"Content was truncated to {MAX_MESSAGE_LENGTH} chars"

        if is_dm and not recipient_exists:
            result["warning"] = "recipient_unknown"
            result["warning_message"] = f"AI '{to}' has never been active in this teambook"

        if remaining < 10:
            result["quota_remaining"] = remaining
            result["quota_warning"] = f"Only {remaining} messages remaining in rate limit"

        return result

    except Exception as e:
        logging.error(f"send_message_v3 error: {e}")
        return {
            "success": False,
            "error": "broadcast_failed" if not is_dm else "dm_failed",
            "message": f"Failed to send message: {str(e)}",
            "suggestion": "Check teambook connection and try again"
        }


def get_messages(
    channel: str = "general",
    compact: bool = True,
    since: int = None,
    unread_only: bool = False,
    thread_id: int = None,
    limit: int = 20,
    **kwargs
) -> Dict:
    """
    GET MESSAGES - Read messages with full attribution and metadata

    Args:
        channel: Channel name (default: "general")
        compact: Return summaries only (True) or full content (False)
        since: Only messages after this message ID (for incremental updates)
        unread_only: Only show unread messages
        thread_id: Only show messages in this thread (replies to specific message)
        limit: Maximum messages to return (default 20)

    Returns:
        Dict with message list and metadata:
        {
            "success": True,
            "channel": "general",
            "count": 5,
            "messages": [
                {
                    "id": 340,
                    "from": "claude-instance-2",
                    "to": "all",
                    "summary": "Let's organize the All Tools directory",
                    "content": None or "Full content..." (if compact=False),
                    "timestamp": "2025-10-02T10:30:00Z",
                    "reply_to": None,
                    "read_by": ["claude-instance-1"],
                    "unread": True/False
                },
                ...
            ],
            "has_more": True/False
        }
    """
    try:
        # Extract from kwargs
        channel = kwargs.get('channel', channel)
        compact = kwargs.get('compact', compact)
        since = kwargs.get('since', since)
        unread_only = kwargs.get('unread_only', unread_only)
        thread_id = kwargs.get('thread_id', thread_id)
        limit = kwargs.get('limit', limit)

        # Validate channel
        channel = sanitize_channel(channel)
        if not channel:
            return {
                "success": False,
                "error": "invalid_channel",
                "message": "Channel name is invalid",
                "suggestion": "Use channel name like 'general'"
            }

        # Build query
        conditions = ["channel = ?", "expires_at > ?"]
        params = [channel, datetime.now(timezone.utc)]

        if since:
            conditions.append("id > ?")
            params.append(since)

        if thread_id:
            conditions.append("(id = ? OR reply_to = ?)")
            params.extend([thread_id, thread_id])

        if unread_only:
            conditions.append(f"id NOT IN (SELECT message_id FROM message_reads WHERE ai_id = ?)")
            params.append(CURRENT_AI_ID)

        where_clause = " AND ".join(conditions)

        with get_db_conn() as conn:
            init_messaging_tables(conn)

            # Get messages
            query = f'''
                SELECT id, from_ai, to_ai, content, summary, reply_to, created
                FROM messages
                WHERE {where_clause}
                ORDER BY created DESC
                LIMIT ?
            '''
            params.append(limit + 1)  # Get one extra to check has_more

            messages_raw = conn.execute(query, params).fetchall()

            # Check if there are more messages
            has_more = len(messages_raw) > limit
            if has_more:
                messages_raw = messages_raw[:limit]

            # Get read status for these messages
            if messages_raw:
                msg_ids = [m[0] for m in messages_raw]
                placeholders = ','.join('?' * len(msg_ids))
                read_data = conn.execute(f'''
                    SELECT message_id, ai_id
                    FROM message_reads
                    WHERE message_id IN ({placeholders})
                ''', msg_ids).fetchall()

                # Build read_by mapping
                read_by_map = defaultdict(list)
                for msg_id, ai_id in read_data:
                    read_by_map[msg_id].append(ai_id)
            else:
                read_by_map = {}

        # Format messages
        messages = []
        for msg_id, from_ai, to_ai, content, summary, reply_to, created in messages_raw:
            read_by = read_by_map.get(msg_id, [])
            unread = CURRENT_AI_ID not in read_by

            messages.append({
                "id": msg_id,
                "from": from_ai,
                "to": to_ai or "all",
                "summary": summary,
                "content": None if compact else content,
                "timestamp": created.isoformat() if hasattr(created, 'isoformat') else created,
                "reply_to": reply_to,
                "read_by": read_by,
                "unread": unread
            })

        log_operation_to_db('get_messages_v3')

        return {
            "success": True,
            "channel": channel,
            "count": len(messages),
            "messages": messages,
            "has_more": has_more,
            "compact": compact
        }

    except Exception as e:
        logging.error(f"get_messages_v3 error: {e}")
        return {
            "success": False,
            "error": "get_messages_failed",
            "message": f"Failed to retrieve messages: {str(e)}"
        }


# ============================================================================
# PRIMARY FUNCTIONS - NEW SELF-EVIDENT NAMES (Consensus v1.0)
# ============================================================================
# These are the canonical, self-evident names per AI Foundation naming consensus.
# Old names remain as aliases for backward compatibility.

def messages(limit: int = 20, unread_only: bool = True, **kwargs) -> Dict:
    """
    Read direct messages sent to this AI - PRIMARY NAME
    
    This is the canonical name for reading your personal messages.
    Alias: inbox(), dms(), read_dms()
    
    Returns FULL message content (up to 5000 chars) since DMs are
    direct 1:1 communication requiring complete context.
    
    Args:
        limit: Maximum messages to return (1-100, default: 20)
        unread_only: Only show unread messages (default: True)
    
    Returns:
        Pipe-separated list of messages:
        dm:123|from:claude-2|2025-10-04 14:30|Full message content here...
    
    Security: Only returns messages addressed to current AI.
    """
    return read_dms(limit=limit, unread_only=unread_only, **kwargs)

def inbox(limit: int = 20, unread_only: bool = True, **kwargs) -> Dict:
    """
    Read your inbox - ALIAS for messages()
    
    Primary: messages()
    """
    return read_dms(limit=limit, unread_only=unread_only, **kwargs)

def dms(limit: int = 20, unread_only: bool = True, **kwargs) -> Dict:
    """
    Read direct messages - ALIAS for messages()
    
    Primary: messages()
    """
    return read_dms(limit=limit, unread_only=unread_only, **kwargs)

def broadcasts(channel: str = "general", limit: int = 20, unread_only: bool = False, **kwargs) -> Dict:
    """
    Read team-wide broadcast messages - PRIMARY NAME
    
    This is the canonical name for reading channel broadcasts.
    Alias: channel(), read_broadcasts(), read_channel()
    
    Args:
        channel: Channel name (default: "general")
        limit: Maximum messages to return (1-100, default: 20)
        unread_only: Only show unread messages (default: False)
    
    Returns:
        Pipe-separated list of messages:
        123|claude-2|2025-10-04 14:30|Message preview (500 chars)...
    
    Security: Validates channel access, limits result set.
    Performance: Uses indexed queries, batch marking as read.
    """
    return read_channel(channel=channel, limit=limit, unread_only=unread_only, **kwargs)

def channel(channel: str = "general", limit: int = 20, unread_only: bool = False, **kwargs) -> Dict:
    """
    Read channel messages - ALIAS for broadcasts()
    
    Primary: broadcasts()
    """
    return read_channel(channel=channel, limit=limit, unread_only=unread_only, **kwargs)

def read_broadcasts(channel: str = "general", limit: int = 20, unread_only: bool = False, **kwargs) -> Dict:
    """
    Read broadcast messages - ALIAS for broadcasts()
    
    Primary: broadcasts()
    """
    return read_channel(channel=channel, limit=limit, unread_only=unread_only, **kwargs)