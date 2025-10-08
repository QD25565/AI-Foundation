"""
PostgreSQL storage backend for Teambook.

This provides a PostgreSQL implementation matching the storage adapter interface,
eliminating DuckDB file locking issues and enabling massive scalability.
"""

import os
import json
import logging
from datetime import datetime, timezone
from typing import Optional, List, Dict, Any
from contextlib import contextmanager

try:
    import psycopg2
    from psycopg2 import pool, sql
    from psycopg2.extras import RealDictCursor, execute_values
    POSTGRES_AVAILABLE = True
except ImportError:
    POSTGRES_AVAILABLE = False

logger = logging.getLogger(__name__)


class PostgresConnectionPool:
    """
    Manages PostgreSQL connection pooling for concurrent access.
    """

    def __init__(self, database: str, user: str, password: str,
                 host: str = 'localhost', port: int = 5432,
                 min_conn: int = 1, max_conn: int = 10):
        """Initialize connection pool."""
        self.pool = psycopg2.pool.ThreadedConnectionPool(
            min_conn, max_conn,
            database=database,
            user=user,
            password=password,
            host=host,
            port=port
        )
        logger.info(f"PostgreSQL connection pool created: {min_conn}-{max_conn} connections")

    @contextmanager
    def get_connection(self):
        """Context manager for getting connections from pool."""
        conn = self.pool.getconn()
        try:
            yield conn
            conn.commit()
        except Exception as e:
            conn.rollback()
            logger.error(f"Transaction failed: {e}")
            raise
        finally:
            self.pool.putconn(conn)

    def close_all(self):
        """Close all connections in pool."""
        self.pool.closeall()


class PostgresTeambookStorage:
    """
    PostgreSQL storage backend for Teambook.

    Implements the same interface as DuckDBTeambookStorage and RedisTeambookStorage.
    """

    def __init__(self, teambook_name: str):
        self.teambook_name = teambook_name
        self._pool = None
        self._init_connection_pool()
        self._ensure_schema()

    def _init_connection_pool(self):
        """Initialize PostgreSQL connection pool."""
        # Get connection details from environment or defaults
        db_name = os.getenv('POSTGRES_DB', 'ai_foundation')
        db_user = os.getenv('POSTGRES_USER', 'ai_foundation')
        db_password = os.getenv('POSTGRES_PASSWORD', 'ai_foundation')
        db_host = os.getenv('POSTGRES_HOST', 'localhost')
        db_port = int(os.getenv('POSTGRES_PORT', '5432'))

        self._pool = PostgresConnectionPool(
            database=db_name,
            user=db_user,
            password=db_password,
            host=db_host,
            port=db_port,
            min_conn=2,
            max_conn=20  # Support high concurrency
        )
        logger.info(f"PostgreSQL storage initialized for teambook: {self.teambook_name}")

    def _ensure_schema(self):
        """Create database schema if it doesn't exist."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                # Notes table with PostgreSQL native types
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS notes (
                        id BIGSERIAL PRIMARY KEY,
                        content TEXT,
                        summary TEXT,
                        tags TEXT[],
                        pinned BOOLEAN DEFAULT FALSE,
                        author VARCHAR(255) NOT NULL,
                        owner VARCHAR(255),
                        teambook_name VARCHAR(255),
                        type VARCHAR(100),
                        parent_id BIGINT,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        session_id BIGINT,
                        linked_items TEXT,
                        pagerank DOUBLE PRECISION DEFAULT 0.0,
                        has_vector BOOLEAN DEFAULT FALSE,
                        status VARCHAR(100),
                        claimed_by VARCHAR(255),
                        assigned_to VARCHAR(255),
                        metadata JSONB
                    )
                """)

                # Edges table for graph relationships
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS edges (
                        from_id BIGINT NOT NULL,
                        to_id BIGINT NOT NULL,
                        type VARCHAR(100) NOT NULL,
                        weight DOUBLE PRECISION DEFAULT 1.0,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        PRIMARY KEY(from_id, to_id, type)
                    )
                """)

                # Evolution outputs
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS evolution_outputs (
                        id BIGSERIAL PRIMARY KEY,
                        evolution_id BIGINT NOT NULL,
                        output_path TEXT NOT NULL,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        author VARCHAR(255) NOT NULL
                    )
                """)

                # Teambooks registry
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS teambooks (
                        name VARCHAR(255) PRIMARY KEY,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        created_by VARCHAR(255) NOT NULL,
                        last_active TIMESTAMP WITH TIME ZONE
                    )
                """)

                # Entities
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS entities (
                        id BIGSERIAL PRIMARY KEY,
                        name VARCHAR(255) UNIQUE NOT NULL,
                        type VARCHAR(100) NOT NULL,
                        first_seen TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        last_seen TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        mention_count INTEGER DEFAULT 1
                    )
                """)

                # Entity-note relationships
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS entity_notes (
                        entity_id BIGINT NOT NULL,
                        note_id BIGINT NOT NULL,
                        PRIMARY KEY(entity_id, note_id)
                    )
                """)

                # Sessions
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS sessions (
                        id BIGSERIAL PRIMARY KEY,
                        started TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        ended TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        note_count INTEGER DEFAULT 1,
                        coherence_score DOUBLE PRECISION DEFAULT 1.0
                    )
                """)

                # Vault (encrypted storage)
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS vault (
                        key VARCHAR(255) PRIMARY KEY,
                        encrypted_value BYTEA NOT NULL,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        updated TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        author VARCHAR(255) NOT NULL
                    )
                """)

                # Messages (for teambook communication)
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS messages (
                        id BIGSERIAL PRIMARY KEY,
                        channel VARCHAR(50) NOT NULL,
                        from_ai VARCHAR(100) NOT NULL,
                        to_ai VARCHAR(100),
                        content TEXT NOT NULL,
                        summary TEXT,
                        reply_to BIGINT,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        read BOOLEAN DEFAULT FALSE,
                        expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
                        teambook_name VARCHAR(50)
                    )
                """)

                # Stats
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS stats (
                        id BIGSERIAL PRIMARY KEY,
                        operation VARCHAR(100) NOT NULL,
                        ts TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        dur_ms INTEGER,
                        author VARCHAR(255)
                    )
                """)

                # Coordination Events (for ambient awareness)
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS coordination_events (
                        id BIGSERIAL PRIMARY KEY,
                        timestamp TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        event_type VARCHAR(50) NOT NULL,
                        ai_id VARCHAR(255) NOT NULL,
                        project_id BIGINT,
                        task_id BIGINT,
                        summary TEXT,
                        metadata JSONB
                    )
                """)

                # AI Last Seen (for temporal continuity)
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS ai_last_seen (
                        ai_id VARCHAR(255) PRIMARY KEY,
                        last_ambient_check TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        last_full_sync TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
                    )
                """)

                # Detangle Rooms (for conflict resolution)
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS detangle_rooms (
                        id BIGSERIAL PRIMARY KEY,
                        room_name VARCHAR(255) UNIQUE NOT NULL,
                        ai_1 VARCHAR(255) NOT NULL,
                        ai_2 VARCHAR(255) NOT NULL,
                        conflict_type VARCHAR(50) NOT NULL,
                        conflict_description TEXT,
                        task_id BIGINT,
                        project_id BIGINT,
                        created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        resolved TIMESTAMP WITH TIME ZONE,
                        resolution_summary TEXT,
                        status VARCHAR(20) NOT NULL DEFAULT 'active',
                        current_turn VARCHAR(255)
                    )
                """)

                # Detangle Messages
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS detangle_messages (
                        id BIGSERIAL PRIMARY KEY,
                        room_id BIGINT NOT NULL,
                        ai_id VARCHAR(255) NOT NULL,
                        message TEXT NOT NULL,
                        timestamp TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        turn_number INTEGER NOT NULL
                    )
                """)

                # Detangle Votes
                cur.execute("""
                    CREATE TABLE IF NOT EXISTS detangle_votes (
                        room_id BIGINT NOT NULL,
                        ai_id VARCHAR(255) NOT NULL,
                        vote VARCHAR(20) NOT NULL,
                        timestamp TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                        PRIMARY KEY (room_id, ai_id)
                    )
                """)

                # Create indexes for performance
                indexes = [
                    "CREATE INDEX IF NOT EXISTS idx_notes_created ON notes(created DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_notes_pinned ON notes(pinned DESC, created DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_notes_pagerank ON notes(pagerank DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_notes_owner ON notes(owner)",
                    "CREATE INDEX IF NOT EXISTS idx_notes_type ON notes(type)",
                    "CREATE INDEX IF NOT EXISTS idx_notes_parent ON notes(parent_id)",
                    "CREATE INDEX IF NOT EXISTS idx_notes_teambook ON notes(teambook_name)",
                    "CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id)",
                    "CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id)",
                    # Message indexes
                    "CREATE INDEX IF NOT EXISTS idx_msg_channel ON messages(channel, created DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_msg_to_ai ON messages(to_ai, read, created DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_msg_expires ON messages(expires_at)",
                    "CREATE INDEX IF NOT EXISTS idx_msg_from_ai ON messages(from_ai, created DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_msg_teambook ON messages(teambook_name)",
                    # Coordination events indexes
                    "CREATE INDEX IF NOT EXISTS idx_events_timestamp ON coordination_events(timestamp DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_events_ai ON coordination_events(ai_id, timestamp DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_events_type ON coordination_events(event_type)",
                    "CREATE INDEX IF NOT EXISTS idx_events_task ON coordination_events(task_id)",
                    "CREATE INDEX IF NOT EXISTS idx_events_project ON coordination_events(project_id)",
                    # Detangle indexes
                    "CREATE INDEX IF NOT EXISTS idx_detangle_status ON detangle_rooms(status, created DESC)",
                    "CREATE INDEX IF NOT EXISTS idx_detangle_ai1 ON detangle_rooms(ai_1)",
                    "CREATE INDEX IF NOT EXISTS idx_detangle_ai2 ON detangle_rooms(ai_2)",
                    "CREATE INDEX IF NOT EXISTS idx_detangle_msgs_room ON detangle_messages(room_id, turn_number)",
                    # GIN indexes for full-text search
                    "CREATE INDEX IF NOT EXISTS idx_notes_content_gin ON notes USING gin(to_tsvector('english', content))",
                    "CREATE INDEX IF NOT EXISTS idx_notes_summary_gin ON notes USING gin(to_tsvector('english', summary))",
                ]

                for idx_sql in indexes:
                    try:
                        cur.execute(idx_sql)
                    except Exception as e:
                        logger.debug(f"Index creation skipped (may already exist): {e}")

                # Add columns for existing tables (migrations)
                migrations = [
                    "ALTER TABLE notes ADD COLUMN IF NOT EXISTS status VARCHAR(100)",
                    "ALTER TABLE notes ADD COLUMN IF NOT EXISTS claimed_by VARCHAR(255)",
                    "ALTER TABLE notes ADD COLUMN IF NOT EXISTS assigned_to VARCHAR(255)",
                    "ALTER TABLE notes ADD COLUMN IF NOT EXISTS metadata JSONB",
                ]

                for migration_sql in migrations:
                    try:
                        cur.execute(migration_sql)
                    except Exception as e:
                        logger.debug(f"Migration skipped (column may already exist): {e}")

                logger.info("PostgreSQL schema verified/created")

    # ==================== NOTES ====================

    def write_note(self, content: str, **kwargs) -> int:
        """Write a note and return its ID."""
        summary = kwargs.get('summary')
        tags = kwargs.get('tags', [])
        author = kwargs.get('author', 'unknown')
        owner = kwargs.get('owner')
        note_type = kwargs.get('note_type', 'general')
        parent_id = kwargs.get('parent_id')
        session_id = kwargs.get('session_id')
        linked_items = kwargs.get('linked_items')
        pinned = kwargs.get('pinned', False)

        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    INSERT INTO notes (
                        content, summary, tags, author, owner, teambook_name,
                        type, parent_id, session_id, linked_items, pinned, created
                    ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                    RETURNING id
                """, (
                    content, summary, tags, author, owner, self.teambook_name,
                    note_type, parent_id, session_id, linked_items, pinned,
                    datetime.now(timezone.utc)
                ))
                note_id = cur.fetchone()[0]
                logger.debug(f"Created note {note_id} in teambook {self.teambook_name}")
                return note_id

    def read_notes(self, **kwargs) -> List[Dict[str, Any]]:
        """Read notes with filtering options."""
        limit = kwargs.get('limit', 20)
        offset = kwargs.get('offset', 0)
        note_type = kwargs.get('note_type')
        author = kwargs.get('author')
        owner = kwargs.get('owner')
        parent_id = kwargs.get('parent_id')
        session_id = kwargs.get('session_id')
        pinned_only = kwargs.get('pinned_only', False)

        conditions = ["teambook_name = %s"]
        params = [self.teambook_name]

        if note_type:
            conditions.append("type = %s")
            params.append(note_type)

        if author:
            conditions.append("author = %s")
            params.append(author)

        if owner:
            conditions.append("owner = %s")
            params.append(owner)

        if parent_id is not None:
            conditions.append("parent_id = %s")
            params.append(parent_id)

        if session_id:
            conditions.append("session_id = %s")
            params.append(session_id)

        if pinned_only:
            conditions.append("pinned = TRUE")

        where_clause = " AND ".join(conditions)
        params.extend([limit, offset])

        with self._pool.get_connection() as conn:
            with conn.cursor(cursor_factory=RealDictCursor) as cur:
                cur.execute(f"""
                    SELECT id, content, summary, tags, pinned, author, owner,
                           type, parent_id, created, session_id, linked_items, pagerank,
                           status, claimed_by, assigned_to, metadata
                    FROM notes
                    WHERE {where_clause}
                    ORDER BY created DESC
                    LIMIT %s OFFSET %s
                """, params)
                return [dict(row) for row in cur.fetchall()]

    def get_note(self, note_id: int) -> Optional[Dict[str, Any]]:
        """Get a single note by ID."""
        with self._pool.get_connection() as conn:
            with conn.cursor(cursor_factory=RealDictCursor) as cur:
                cur.execute("""
                    SELECT id, content, summary, tags, pinned, author, owner,
                           type, parent_id, created, session_id, linked_items, pagerank,
                           status, claimed_by, assigned_to, metadata
                    FROM notes
                    WHERE id = %s AND teambook_name = %s
                """, (note_id, self.teambook_name))
                row = cur.fetchone()
                return dict(row) if row else None

    def update_note(self, note_id: int, **updates) -> bool:
        """Update note fields."""
        if not updates:
            return False

        set_clauses = []
        params = []

        for field, value in updates.items():
            set_clauses.append(f"{field} = %s")
            params.append(value)

        params.extend([note_id, self.teambook_name])

        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute(f"""
                    UPDATE notes
                    SET {', '.join(set_clauses)}
                    WHERE id = %s AND teambook_name = %s
                """, params)
                return cur.rowcount > 0

    def delete_note(self, note_id: int) -> bool:
        """Delete a note."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    DELETE FROM notes
                    WHERE id = %s AND teambook_name = %s
                """, (note_id, self.teambook_name))
                return cur.rowcount > 0

    # ==================== EDGES ====================

    def add_edge(self, from_id: int, to_id: int, edge_type: str, weight: float = 1.0) -> None:
        """Add a graph edge."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    INSERT INTO edges (from_id, to_id, type, weight, created)
                    VALUES (%s, %s, %s, %s, %s)
                    ON CONFLICT (from_id, to_id, type) DO NOTHING
                """, (from_id, to_id, edge_type, weight, datetime.now(timezone.utc)))

    def get_edges(self, note_id: int, reverse: bool = False) -> List[Dict[str, Any]]:
        """Get edges from/to a note."""
        field = "to_id" if reverse else "from_id"

        with self._pool.get_connection() as conn:
            with conn.cursor(cursor_factory=RealDictCursor) as cur:
                cur.execute(f"""
                    SELECT from_id, to_id, type, weight, created
                    FROM edges
                    WHERE {field} = %s
                """, (note_id,))
                return [dict(row) for row in cur.fetchall()]

    # ==================== VAULT ====================

    def vault_set(self, key: str, encrypted_value: str, author: str) -> None:
        """Store encrypted value."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                now = datetime.now(timezone.utc)
                cur.execute("""
                    INSERT INTO vault (key, encrypted_value, created, updated, author)
                    VALUES (%s, %s, %s, %s, %s)
                    ON CONFLICT (key) DO UPDATE
                    SET encrypted_value = EXCLUDED.encrypted_value,
                        updated = EXCLUDED.updated,
                        author = EXCLUDED.author
                """, (key, encrypted_value.encode(), now, now, author))

    def vault_get(self, key: str) -> Optional[str]:
        """Retrieve encrypted value."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    SELECT encrypted_value FROM vault WHERE key = %s
                """, (key,))
                row = cur.fetchone()
                if row:
                    # Handle both bytes and memoryview
                    encrypted = row[0]
                    if isinstance(encrypted, memoryview):
                        encrypted = bytes(encrypted)
                    return encrypted.decode()
                return None

    def vault_delete(self, key: str) -> bool:
        """Delete encrypted value."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("DELETE FROM vault WHERE key = %s", (key,))
                return cur.rowcount > 0

    # ==================== SESSIONS ====================

    def create_session(self) -> int:
        """Create a new session."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                now = datetime.now(timezone.utc)
                cur.execute("""
                    INSERT INTO sessions (started, ended)
                    VALUES (%s, %s)
                    RETURNING id
                """, (now, now))
                return cur.fetchone()[0]

    # ==================== STATS ====================

    def get_stats(self) -> Dict[str, int]:
        """Get teambook statistics."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                stats = {}

                # Note count
                cur.execute("""
                    SELECT COUNT(*) FROM notes WHERE teambook_name = %s
                """, (self.teambook_name,))
                stats['notes'] = cur.fetchone()[0]

                # Edge count (approximate - edges don't have teambook_name)
                cur.execute("SELECT COUNT(*) FROM edges")
                stats['edges'] = cur.fetchone()[0]

                # Entity count
                cur.execute("SELECT COUNT(*) FROM entities")
                stats['entities'] = cur.fetchone()[0]

                # Session count
                cur.execute("SELECT COUNT(*) FROM sessions")
                stats['sessions'] = cur.fetchone()[0]

                return stats

    # ==================== MESSAGING ====================

    def write_message(self, channel: str, from_ai: str, content: str, **kwargs) -> int:
        """Write a message to a channel or DM and return its ID."""
        to_ai = kwargs.get('to_ai')
        summary = kwargs.get('summary')
        reply_to = kwargs.get('reply_to')
        expires_at = kwargs.get('expires_at')

        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    INSERT INTO messages (
                        channel, from_ai, to_ai, content, summary, reply_to,
                        created, expires_at, teambook_name
                    ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s)
                    RETURNING id
                """, (
                    channel, from_ai, to_ai, content, summary, reply_to,
                    datetime.now(timezone.utc), expires_at, self.teambook_name
                ))
                msg_id = cur.fetchone()[0]
                logger.debug(f"Created message {msg_id} in channel {channel}")
                return msg_id

    def read_messages(self, channel: str = None, to_ai: str = None, **kwargs) -> List[Dict[str, Any]]:
        """Read messages from a channel or DMs."""
        limit = kwargs.get('limit', 20)
        unread_only = kwargs.get('unread_only', False)
        from_ai = kwargs.get('from_ai')

        conditions = ["teambook_name = %s"]
        params = [self.teambook_name]

        if channel:
            conditions.append("channel = %s")
            params.append(channel)

        if to_ai:
            conditions.append("to_ai = %s")
            params.append(to_ai)

        if from_ai:
            conditions.append("from_ai = %s")
            params.append(from_ai)

        if unread_only:
            conditions.append("read = FALSE")

        # Filter out expired messages
        conditions.append("expires_at > %s")
        params.append(datetime.now(timezone.utc))

        where_clause = " AND ".join(conditions)
        params.append(limit)

        with self._pool.get_connection() as conn:
            with conn.cursor(cursor_factory=RealDictCursor) as cur:
                cur.execute(f"""
                    SELECT id, channel, from_ai, to_ai, content, summary, reply_to,
                           created, read, expires_at
                    FROM messages
                    WHERE {where_clause}
                    ORDER BY created DESC
                    LIMIT %s
                """, params)
                return [dict(row) for row in cur.fetchall()]

    def mark_messages_read(self, message_ids: List[int]) -> int:
        """Mark messages as read. Returns count of updated messages."""
        if not message_ids:
            return 0

        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    UPDATE messages
                    SET read = TRUE
                    WHERE id = ANY(%s) AND teambook_name = %s
                """, (message_ids, self.teambook_name))
                return cur.rowcount

    def cleanup_expired_messages(self) -> int:
        """Delete expired messages. Returns count of deleted messages."""
        with self._pool.get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("""
                    DELETE FROM messages
                    WHERE teambook_name = %s AND expires_at < %s
                """, (self.teambook_name, datetime.now(timezone.utc)))
                deleted = cur.rowcount
                if deleted > 0:
                    logger.info(f"Cleaned up {deleted} expired messages")
                return deleted

    def close(self):
        """Close all database connections."""
        if self._pool:
            self._pool.close_all()
