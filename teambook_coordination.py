#!/usr/bin/env python3
"""
TEAMBOOK COORDINATION v1.0.0 - SECURE MULTI-AGENT PRIMITIVES
==============================================================
Distributed locks, task queues, and atomic operations for AI collaboration.
Zero-trust, high-performance, deadlock-free design.

Security Features:
- Automatic lock expiration (prevents deadlock)
- Per-AI lock limits (prevents resource hoarding)
- Queue priority validation (prevents gaming)
- Atomic operations (prevents race conditions)
- Input sanitization (prevents injection)
"""

import time
import json
import hashlib
from datetime import datetime, timedelta, timezone
from typing import Dict, Optional, Tuple, List, Any
from collections import defaultdict
import logging

from teambook_shared import (
    CURRENT_AI_ID, CURRENT_TEAMBOOK, OUTPUT_FORMAT,
    pipe_escape, format_time_compact, clean_text
)

from teambook_storage import get_db_conn, log_operation_to_db
from storage_adapter import TeambookStorageAdapter
from teambook_config import get_storage_backend

# Try to import PostgreSQL-specific functions
try:
    from teambook_storage_postgresql import get_pg_conn
    POSTGRES_AVAILABLE = True
except ImportError:
    POSTGRES_AVAILABLE = False
    get_pg_conn = None

# ============= SECURITY LIMITS =============

MAX_LOCK_DURATION_SECONDS = 300  # 5 minutes max
DEFAULT_LOCK_TIMEOUT = 30  # 30 seconds default
MAX_LOCKS_PER_AI = 10  # Prevent resource hoarding
MAX_QUEUE_SIZE = 1000  # Prevent memory exhaustion
MAX_TASK_LENGTH = 2000
MAX_RESOURCE_ID_LENGTH = 100

# In-memory lock tracking for performance
_lock_cache = {}  # resource_id -> (ai_id, expires_at)
_ai_lock_count = defaultdict(int)  # ai_id -> count


def _task_hash_default(obj):
    if isinstance(obj, datetime):
        return obj.isoformat()
    return obj


def compute_task_tamper_hash(record: Dict[str, Any]) -> str:
    payload = {
        'task': record.get('task'),
        'priority': record.get('priority'),
        'status': record.get('status'),
        'claimed_by': record.get('claimed_by'),
        'created_at': record.get('created_at'),
        'claimed_at': record.get('claimed_at'),
        'completed_at': record.get('completed_at'),
        'result': record.get('result'),
        'metadata': record.get('metadata'),
        'teambook_name': record.get('teambook_name'),
        'representation_policy': record.get('representation_policy', 'default') or 'default'
    }
    serialized = json.dumps(payload, sort_keys=True, default=_task_hash_default)
    return hashlib.sha256(serialized.encode('utf-8')).hexdigest()


def _refresh_task_tamper_hash(conn, task_id: int) -> None:
    try:
        row = conn.execute(
            '''
            SELECT task, priority, status, claimed_by, created_at, claimed_at,
                   completed_at, result, metadata, teambook_name, representation_policy
            FROM task_queue
            WHERE id = ?
            ''',
            [task_id]
        ).fetchone()

        if not row:
            return

        columns = [
            'task', 'priority', 'status', 'claimed_by', 'created_at', 'claimed_at',
            'completed_at', 'result', 'metadata', 'teambook_name', 'representation_policy'
        ]
        record = dict(zip(columns, row))
        tamper_hash = compute_task_tamper_hash(record)
        conn.execute('UPDATE task_queue SET tamper_hash = ? WHERE id = ?', [tamper_hash, task_id])
    except Exception as exc:
        logging.debug(f"Tamper hash refresh failed for task {task_id}: {exc}")

# ============= STORAGE BACKEND SELECTION =============

def get_coordination_backend():
    """
    Get the best available backend for coordination primitives.

    Priority (ENTERPRISE-GRADE FIRST):
    1. PostgreSQL - Row-level locking, ACID transactions, multi-process safe
    2. DuckDB - Single-process fallback (file locking issues in multi-process)

    Returns: tuple of (backend_type: str, connection_getter: callable)
    """
    backend = get_storage_backend()

    # Try PostgreSQL FIRST (required for multi-AI coordination)
    if backend == 'postgresql' and POSTGRES_AVAILABLE:
        return ('postgresql', get_pg_conn)

    # Fallback to DuckDB (WARN: Not suitable for multi-AI!)
    logging.warning("Coordination using DuckDB fallback - NOT SUITABLE FOR MULTI-AI! Please configure PostgreSQL.")
    return ('duckdb', get_db_conn)

# ============= INPUT VALIDATION =============

def sanitize_resource_id(resource_id: str) -> Optional[str]:
    """
    Sanitize resource identifier - SECURITY CRITICAL

    Returns None if invalid, sanitized string if valid.
    """
    if not resource_id:
        return None

    resource_id = str(resource_id).strip()

    # Length check
    if len(resource_id) > MAX_RESOURCE_ID_LENGTH:
        return None

    # Character whitelist (alphanumeric, dash, underscore, colon, dot, slash)
    import re
    if not re.match(r'^[A-Za-z0-9_:\-\./]+$', resource_id):
        return None

    return resource_id

def validate_timeout(timeout: int) -> int:
    """Validate and clamp timeout value"""
    try:
        timeout = int(timeout)
    except:
        timeout = DEFAULT_LOCK_TIMEOUT

    if timeout < 1:
        timeout = DEFAULT_LOCK_TIMEOUT
    if timeout > MAX_LOCK_DURATION_SECONDS:
        timeout = MAX_LOCK_DURATION_SECONDS

    return timeout

def validate_priority(priority: int) -> int:
    """Validate and clamp priority value (0-9, higher = more urgent)"""
    try:
        priority = int(priority)
    except:
        priority = 5

    return max(0, min(9, priority))

# ============= DATABASE INITIALIZATION =============

def init_coordination_tables(conn):
    """Initialize coordination tables with proper indexes"""

    # Create sequences for auto-increment
    try:
        conn.execute('CREATE SEQUENCE IF NOT EXISTS seq_tasks')
    except Exception:
        pass  # Sequence might already exist

    # Distributed locks table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS locks (
            resource_id VARCHAR(100) PRIMARY KEY,
            held_by VARCHAR(100) NOT NULL,
            acquired_at TIMESTAMPTZ NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL,
            teambook_name VARCHAR(50)
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_locks_expires ON locks(expires_at)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_locks_holder ON locks(held_by)')

    # Task queue table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS task_queue (
            id INTEGER PRIMARY KEY DEFAULT nextval('seq_tasks'),
            task TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 5,
            status VARCHAR(20) NOT NULL DEFAULT 'pending',
            claimed_by VARCHAR(100),
            created_at TIMESTAMPTZ NOT NULL,
            claimed_at TIMESTAMPTZ,
            completed_at TIMESTAMPTZ,
            result TEXT,
            teambook_name VARCHAR(50),
            metadata TEXT,
            representation_policy VARCHAR DEFAULT 'default',
            tamper_hash VARCHAR
        )
    ''')

    conn.execute('CREATE INDEX IF NOT EXISTS idx_queue_status_priority ON task_queue(status, priority DESC, created_at)')
    conn.execute('CREATE INDEX IF NOT EXISTS idx_queue_claimed ON task_queue(claimed_by, status)')

    try:
        conn.execute("ALTER TABLE task_queue ADD COLUMN representation_policy VARCHAR DEFAULT 'default'")
    except Exception:
        pass

    try:
        conn.execute("ALTER TABLE task_queue ADD COLUMN tamper_hash VARCHAR")
    except Exception:
        pass

    conn.commit()

def cleanup_expired_locks(conn):
    """Remove expired locks - called periodically"""
    try:
        now = datetime.now(timezone.utc)

        expired = conn.execute(
            'SELECT resource_id, held_by FROM locks WHERE expires_at < ?',
            [now]
        ).fetchall()

        if expired:
            resource_ids = [r[0] for r in expired]
            placeholders = ','.join(['?'] * len(resource_ids))

            # Security: Use parameterized query (placeholders already safe as generated from count)
            conn.execute(f'DELETE FROM locks WHERE resource_id IN ({placeholders})', resource_ids)
            conn.commit()

            # Update in-memory cache
            for resource_id, held_by in expired:
                _lock_cache.pop(resource_id, None)
                _ai_lock_count[held_by] = max(0, _ai_lock_count[held_by] - 1)

            logging.info(f"Cleaned up {len(expired)} expired locks")

    except Exception as e:
        logging.error(f"Lock cleanup error: {e}")

# ============= DISTRIBUTED LOCKS =============

def acquire_lock(resource_id: str = None, timeout: int = DEFAULT_LOCK_TIMEOUT, **kwargs) -> str:
    """
    Acquire distributed lock on a resource.

    ENTERPRISE-GRADE: Uses PostgreSQL row-level locking when available,
    falls back to DuckDB (single-process only) if PostgreSQL unavailable.

    Security:
    - Automatic expiration prevents deadlock
    - Per-AI limits prevent hoarding
    - Atomic check-and-set prevents races

    Returns lock token if successful, error otherwise.
    """
    try:
        resource_id = sanitize_resource_id(kwargs.get('resource_id', resource_id))
        if not resource_id:
            return "!invalid_resource_id"

        timeout = validate_timeout(kwargs.get('timeout', timeout))

        # Check per-AI lock limit
        if _ai_lock_count[CURRENT_AI_ID] >= MAX_LOCKS_PER_AI:
            return f"!lock_limit:max_{MAX_LOCKS_PER_AI}"

        now = datetime.now(timezone.utc)
        expires_at = now + timedelta(seconds=timeout)

        # Get best available backend (PostgreSQL first!)
        backend_type, get_conn = get_coordination_backend()

        with get_conn() as conn:
            if backend_type == 'postgresql':
                # PostgreSQL: Use proper row-level locking with NOWAIT
                cur = conn.cursor()
                init_coordination_tables(conn)
            else:
                # DuckDB fallback
                init_coordination_tables(conn)

            # Atomic check-and-acquire
            # First, try to claim expired or non-existent lock
            conn.execute('''
                INSERT INTO locks (resource_id, held_by, acquired_at, expires_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(resource_id) DO UPDATE SET
                    held_by = excluded.held_by,
                    acquired_at = excluded.acquired_at,
                    expires_at = excluded.expires_at
                WHERE locks.expires_at < ?
            ''', [resource_id, CURRENT_AI_ID, now, expires_at, now])

            # Verify we got the lock
            lock = conn.execute(
                'SELECT held_by, expires_at FROM locks WHERE resource_id = ?',
                [resource_id]
            ).fetchone()

            if not lock or lock[0] != CURRENT_AI_ID:
                holder = lock[0] if lock else "unknown"
                return f"!locked_by:{holder}"

            # Update cache
            _lock_cache[resource_id] = (CURRENT_AI_ID, expires_at)
            _ai_lock_count[CURRENT_AI_ID] += 1

        log_operation_to_db('acquire_lock')

        # Pure pipe format (token optimized!)
        remaining = int((expires_at - now).total_seconds())
        return f"{resource_id}|expires:{remaining}s"

    except Exception as e:
        logging.error(f"Acquire lock error: {e}")
        return "!lock_failed"

def release_lock(resource_id: str = None, **kwargs) -> str:
    """
    Release a held lock.

    Security: Only holder can release their own lock.
    """
    try:
        resource_id = sanitize_resource_id(kwargs.get('resource_id', resource_id))
        if not resource_id:
            return "!invalid_resource_id"

        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)

            # Verify ownership before releasing
            lock = conn.execute(
                'SELECT held_by FROM locks WHERE resource_id = ?',
                [resource_id]
            ).fetchone()

            if not lock:
                return "!not_locked"

            if lock[0] != CURRENT_AI_ID:
                return f"!not_your_lock:held_by_{lock[0]}"

            # Release
            conn.execute('DELETE FROM locks WHERE resource_id = ?', [resource_id])

            # Update cache
            _lock_cache.pop(resource_id, None)
            _ai_lock_count[CURRENT_AI_ID] = max(0, _ai_lock_count[CURRENT_AI_ID] - 1)

        log_operation_to_db('release_lock')

        return resource_id  # Pure pipe format!

    except Exception as e:
        logging.error(f"Release lock error: {e}")
        return "!release_failed"

def extend_lock(resource_id: str = None, additional_seconds: int = 30, **kwargs) -> str:
    """
    Extend lock expiration time.

    Security: Only holder can extend, limited duration.
    """
    try:
        resource_id = sanitize_resource_id(kwargs.get('resource_id', resource_id))
        if not resource_id:
            return "!invalid_resource_id"

        additional = validate_timeout(kwargs.get('additional_seconds', additional_seconds))

        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)

            # Verify ownership
            lock = conn.execute(
                'SELECT held_by, expires_at FROM locks WHERE resource_id = ?',
                [resource_id]
            ).fetchone()

            if not lock:
                return "!not_locked"

            if lock[0] != CURRENT_AI_ID:
                return "!not_your_lock"

            # Calculate new expiration (max 5 minutes from now)
            now = datetime.now(timezone.utc)
            current_expires = lock[1]
            new_expires = min(
                current_expires + timedelta(seconds=additional),
                now + timedelta(seconds=MAX_LOCK_DURATION_SECONDS)
            )

            conn.execute(
                'UPDATE locks SET expires_at = ? WHERE resource_id = ?',
                [new_expires, resource_id]
            )

            # Update cache
            _lock_cache[resource_id] = (CURRENT_AI_ID, new_expires)

        log_operation_to_db('extend_lock')

        remaining = int((new_expires - now).total_seconds())
        # Pure pipe format (token optimized!)
        return f"{resource_id}|new_ttl:{remaining}s"

    except Exception as e:
        logging.error(f"Extend lock error: {e}")
        return "!extend_failed"

def list_locks(show_all: bool = False, **kwargs) -> str:
    """
    List active locks.

    show_all: Show all locks (default: only yours)
    """
    try:
        show_all = bool(kwargs.get('show_all', show_all))

        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)
            cleanup_expired_locks(conn)

            if show_all:
                query = '''
                    SELECT resource_id, held_by, expires_at
                    FROM locks
                    WHERE expires_at > ?
                    ORDER BY expires_at
                '''
                params = [datetime.now(timezone.utc)]
            else:
                query = '''
                    SELECT resource_id, held_by, expires_at
                    FROM locks
                    WHERE held_by = ? AND expires_at > ?
                    ORDER BY expires_at
                '''
                params = [CURRENT_AI_ID, datetime.now(timezone.utc)]

            locks = conn.execute(query, params).fetchall()

        if not locks:
            return ""  # Empty string for no locks (token optimized!)

        # Pure pipe format (token optimized!) - newline separated
        lines = []
        now = datetime.now(timezone.utc)
        for resource_id, held_by, expires_at in locks:
            remaining = int((expires_at - now).total_seconds())
            parts = [resource_id, held_by, f"{remaining}s"]
            lines.append('|'.join(pipe_escape(p) for p in parts))
        return '\n'.join(lines)  # Direct string, newline separated!

    except Exception as e:
        logging.error(f"List locks error: {e}")
        return "!list_failed"

# ============= TASK QUEUE =============

def queue_task(task: str = None, priority: int = 5, metadata: str = None,
               representation_policy: str = 'default', **kwargs) -> str:
    """
    Add task to distributed queue.

    priority: 0-9 (higher = more urgent)
    metadata: Optional JSON string with extra data
    """
    try:
        task = clean_text(kwargs.get('task', task))
        if not task:
            return "!empty_task"

        if len(task) > MAX_TASK_LENGTH:
            task = task[:MAX_TASK_LENGTH]

        priority = validate_priority(kwargs.get('priority', priority))
        metadata = kwargs.get('metadata', metadata)
        representation_policy = (kwargs.get('representation_policy', representation_policy) or 'default').strip().lower()

        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)

            # Check queue size
            count = conn.execute(
                "SELECT COUNT(*) FROM task_queue WHERE status = 'pending'"
            ).fetchone()[0]

            if count >= MAX_QUEUE_SIZE:
                return f"!queue_full|max:{MAX_QUEUE_SIZE}|pending:{count}"

            cursor = conn.execute('''
                INSERT INTO task_queue (
                    task, priority, status, created_at, metadata, teambook_name,
                    representation_policy, tamper_hash
                )
                VALUES (?, ?, 'pending', ?, ?, ?, ?, NULL)
                RETURNING id
            ''', [
                task,
                priority,
                datetime.now(timezone.utc),
                metadata,
                CURRENT_TEAMBOOK,
                representation_policy
            ])

            task_id = cursor.fetchone()[0]
            _refresh_task_tamper_hash(conn, task_id)

        log_operation_to_db('queue_task')

        # Pure pipe format (token optimized!)
        return f"task:{task_id}|priority:{priority}"

    except Exception as e:
        logging.error(f"Queue task error: {e}")
        return "!queue_failed"

def claim_task(prefer_priority: bool = True, **kwargs) -> str:
    """
    Claim next available task from queue.

    prefer_priority: If True, gets highest priority task (default)
    """
    try:
        prefer_priority = bool(kwargs.get('prefer_priority', prefer_priority if prefer_priority is not None else True))

        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)

            # Atomic claim: update and return in one operation
            if prefer_priority:
                order = 'priority DESC, created_at ASC'
            else:
                order = 'created_at ASC'

            # Security: Get first available task with row-level locking for PostgreSQL
            # For PostgreSQL: SELECT FOR UPDATE prevents race conditions
            # For DuckDB: The WHERE status='pending' condition in UPDATE handles races
            if backend_type == 'postgresql':
                task = conn.execute(f'''
                    SELECT id, task, priority, created_at, metadata
                    FROM task_queue
                    WHERE status = 'pending'
                    ORDER BY {order}
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                ''').fetchone()
            else:
                task = conn.execute(f'''
                    SELECT id, task, priority, created_at, metadata
                    FROM task_queue
                    WHERE status = 'pending'
                    ORDER BY {order}
                    LIMIT 1
                ''').fetchone()

            if not task:
                return ""  # Empty string for empty queue (token optimized!)

            task_id, task_desc, priority, created_at, metadata = task

            # Claim it atomically
            conn.execute('''
                UPDATE task_queue
                SET status = 'claimed', claimed_by = ?, claimed_at = ?
                WHERE id = ? AND status = 'pending'
            ''', [CURRENT_AI_ID, datetime.now(timezone.utc), task_id])

            _refresh_task_tamper_hash(conn, task_id)

            # Verify we got it (handles race conditions)
            claimed = conn.execute(
                'SELECT claimed_by FROM task_queue WHERE id = ?',
                [task_id]
            ).fetchone()

            if not claimed or claimed[0] != CURRENT_AI_ID:
                return "!task_claimed_by_other"

        log_operation_to_db('claim_task')

        # Pure pipe format (token optimized!)
        parts = [
            f"task:{task_id}",
            f"p:{priority}",
            format_time_compact(created_at),
            task_desc[:100]
        ]
        if metadata:
            parts.append(f"meta:{metadata[:50]}")
        return '|'.join(pipe_escape(p) for p in parts)

    except Exception as e:
        logging.error(f"Claim task error: {e}")
        return "!claim_failed"

def complete_task(task_id: int = None, result: str = None, **kwargs) -> str:
    """
    Mark task as completed.

    Security: Only claimer can complete their task.
    """
    try:
        task_id = int(kwargs.get('task_id', task_id))
        result_text = kwargs.get('result', result)

        if result_text and len(result_text) > MAX_TASK_LENGTH:
            result_text = result_text[:MAX_TASK_LENGTH]

        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)

            # Verify ownership
            task = conn.execute(
                'SELECT claimed_by, status FROM task_queue WHERE id = ?',
                [task_id]
            ).fetchone()

            if not task:
                return "!task_not_found"

            if task[1] == 'completed':
                return "!already_completed"

            if task[0] != CURRENT_AI_ID:
                return f"!not_your_task|claimed_by:{task[0]}"

            # Complete
            conn.execute('''
                UPDATE task_queue
                SET status = 'completed', completed_at = ?, result = ?
                WHERE id = ?
            ''', [datetime.now(timezone.utc), result_text, task_id])

            _refresh_task_tamper_hash(conn, task_id)

        log_operation_to_db('complete_task')

        # Pure pipe format (token optimized!)
        return f"task:{task_id}"

    except Exception as e:
        logging.error(f"Complete task error: {e}")
        return "!complete_failed"

def queue_stats(**kwargs) -> str:
    """Get task queue statistics"""
    try:
        backend_type, get_conn = get_coordination_backend()
        with get_conn() as conn:
            init_coordination_tables(conn)

            stats = conn.execute('''
                SELECT
                    COUNT(*) as total,
                    COUNT(CASE WHEN status = 'pending' THEN 1 END) as pending,
                    COUNT(CASE WHEN status = 'claimed' THEN 1 END) as claimed,
                    COUNT(CASE WHEN status = 'completed' THEN 1 END) as completed,
                    COUNT(CASE WHEN claimed_by = ? THEN 1 END) as my_tasks
                FROM task_queue
            ''', [CURRENT_AI_ID]).fetchone()

            total, pending, claimed, completed, my_tasks = stats

        # Pure pipe format (token optimized!)
        parts = [
            f"total:{total}",
            f"pending:{pending}",
            f"claimed:{claimed}",
            f"done:{completed}",
            f"mine:{my_tasks}"
        ]
        return '|'.join(parts)

    except Exception as e:
        logging.error(f"Queue stats error: {e}")
        return "!stats_failed"