"""
Redis-based storage backend for Teambook.

Implements the data model from REDIS_MIGRATION_PLAN.md:
- Hash-based note storage
- Sorted sets for timeline/pagerank
- Sequences for auto-increment IDs
- Graph edges for relationships
- Vault for encrypted secrets
"""

import json
import time
from typing import Optional, List, Dict, Any, Tuple
from datetime import datetime
import logging
import redis

from .redis_pool import get_connection
from .redis_events import publish_event

logger = logging.getLogger(__name__)


class RedisTeambookStorage:
    """Redis storage backend for a Teambook."""

    def __init__(self, teambook_name: str):
        self.teambook_name = teambook_name
        self.prefix = f"teambook:{teambook_name}"

    def _get_conn(self):
        """Get Redis connection from pool."""
        return get_connection()

    # ==================== NOTES ====================

    def write_note(
        self,
        content: str,
        summary: Optional[str] = None,
        tags: Optional[str] = None,
        author: Optional[str] = None,
        owner: Optional[str] = None,
        note_type: str = "general",
        parent_id: Optional[int] = None,
        session_id: Optional[int] = None,
        linked_items: Optional[List[Dict]] = None,
        pinned: bool = False
    ) -> int:
        """
        Write a note to Redis.

        Args:
            content: Note content
            summary: Optional summary
            tags: Optional comma-separated tags
            author: Author AI ID
            owner: Owner AI ID
            note_type: Type of note (general, dm, task, etc.)
            parent_id: Parent note ID for threading
            session_id: Session ID
            linked_items: List of linked items
            pinned: Whether note is pinned

        Returns:
            Note ID
        """
        conn = self._get_conn()

        # Get next note ID
        note_id = conn.incr(f"{self.prefix}:seq:notes")

        # Prepare note data
        created = datetime.utcnow().isoformat()
        note_data = {
            'id': note_id,
            'content': content,
            'summary': summary or '',
            'tags': tags or '',
            'pinned': '1' if pinned else '0',
            'author': author or '',
            'owner': owner or '',
            'type': note_type,
            'parent_id': str(parent_id) if parent_id else '',
            'created': created,
            'session_id': str(session_id) if session_id else '',
            'pagerank': '0.01',  # Default pagerank
            'has_vector': '0',
            'linked_items': json.dumps(linked_items) if linked_items else '[]'
        }

        # Store note as hash (use hmset for Redis 3.x compatibility)
        note_key = f"{self.prefix}:note:{note_id}"
        try:
            # Try modern syntax first (Redis 4.0+)
            conn.hset(note_key, mapping=note_data)
        except redis.exceptions.ResponseError:
            # Fallback to hmset for older Redis (3.x)
            conn.hmset(note_key, note_data)

        # Add to timeline (sorted by timestamp)
        timestamp = time.time()
        conn.zadd(f"{self.prefix}:notes:timeline", {note_id: timestamp})

        # Add to pagerank sorted set
        conn.zadd(f"{self.prefix}:notes:pagerank", {note_id: 0.01})

        # Add to indexes
        if author:
            conn.sadd(f"{self.prefix}:idx:author:{author}", note_id)
        if note_type:
            conn.sadd(f"{self.prefix}:idx:type:{note_type}", note_id)
        if session_id:
            conn.sadd(f"{self.prefix}:idx:session:{session_id}", note_id)

        # Handle pinned
        if pinned:
            conn.sadd(f"{self.prefix}:notes:pinned", note_id)

        # Publish event
        publish_event(self.teambook_name, 'note_created', {
            'note_id': note_id,
            'author': author,
            'type': note_type,
            'content': content[:100]  # First 100 chars
        })

        logger.debug(f"Created note {note_id} in {self.teambook_name}")
        return note_id

    def read_notes(
        self,
        limit: int = 20,
        offset: int = 0,
        note_type: Optional[str] = None,
        author: Optional[str] = None,
        session_id: Optional[int] = None,
        pinned_only: bool = False,
        reverse: bool = True
    ) -> List[Dict[str, Any]]:
        """
        Read notes from Redis.

        Args:
            limit: Maximum number of notes
            offset: Offset for pagination
            note_type: Filter by note type
            author: Filter by author
            session_id: Filter by session
            pinned_only: Only return pinned notes
            reverse: Reverse chronological order (newest first)

        Returns:
            List of note dictionaries
        """
        conn = self._get_conn()

        # Get note IDs based on filters
        if pinned_only:
            note_ids = conn.smembers(f"{self.prefix}:notes:pinned")
            note_ids = sorted(note_ids, key=int, reverse=reverse)
        elif author:
            note_ids = conn.smembers(f"{self.prefix}:idx:author:{author}")
            note_ids = sorted(note_ids, key=int, reverse=reverse)
        elif note_type:
            note_ids = conn.smembers(f"{self.prefix}:idx:type:{note_type}")
            note_ids = sorted(note_ids, key=int, reverse=reverse)
        elif session_id:
            note_ids = conn.smembers(f"{self.prefix}:idx:session:{session_id}")
            note_ids = sorted(note_ids, key=int, reverse=reverse)
        else:
            # Get from timeline sorted set
            if reverse:
                note_ids = conn.zrevrange(f"{self.prefix}:notes:timeline", offset, offset + limit - 1)
            else:
                note_ids = conn.zrange(f"{self.prefix}:notes:timeline", offset, offset + limit - 1)

        # Apply pagination if using sets
        if isinstance(note_ids, (set, list)) and not isinstance(note_ids, range):
            note_ids = list(note_ids)[offset:offset + limit]

        # Fetch note data
        notes = []
        for note_id in note_ids:
            note_key = f"{self.prefix}:note:{note_id}"
            note_data = conn.hgetall(note_key)

            if note_data:
                # Convert string values back to proper types
                note = self._deserialize_note(note_data)
                notes.append(note)

        return notes

    def get_note(self, note_id: int) -> Optional[Dict[str, Any]]:
        """Get a single note by ID."""
        conn = self._get_conn()
        note_key = f"{self.prefix}:note:{note_id}"
        note_data = conn.hgetall(note_key)

        if not note_data:
            return None

        return self._deserialize_note(note_data)

    def update_note(self, note_id: int, **updates) -> bool:
        """Update note fields."""
        conn = self._get_conn()
        note_key = f"{self.prefix}:note:{note_id}"

        if not conn.exists(note_key):
            return False

        # Convert values to strings for Redis hash
        updates_str = {}
        for k, v in updates.items():
            if v is None:
                updates_str[k] = ''
            elif isinstance(v, bool):
                # Convert boolean to '1' or '0' for consistency
                updates_str[k] = '1' if v else '0'
            else:
                updates_str[k] = str(v)

        try:
            # Try modern syntax first (Redis 4.0+)
            conn.hset(note_key, mapping=updates_str)
        except redis.exceptions.ResponseError:
            # Fallback to hmset for older Redis (3.x)
            conn.hmset(note_key, updates_str)

        # Update pinned set if needed
        if 'pinned' in updates:
            if updates['pinned']:
                conn.sadd(f"{self.prefix}:notes:pinned", note_id)
            else:
                conn.srem(f"{self.prefix}:notes:pinned", note_id)

        return True

    def delete_note(self, note_id: int) -> bool:
        """Delete a note."""
        conn = self._get_conn()
        note_key = f"{self.prefix}:note:{note_id}"

        # Get note data for cleanup
        note_data = conn.hgetall(note_key)
        if not note_data:
            return False

        # Remove from all indexes
        conn.zrem(f"{self.prefix}:notes:timeline", note_id)
        conn.zrem(f"{self.prefix}:notes:pagerank", note_id)
        conn.srem(f"{self.prefix}:notes:pinned", note_id)

        if note_data.get('author'):
            conn.srem(f"{self.prefix}:idx:author:{note_data['author']}", note_id)
        if note_data.get('type'):
            conn.srem(f"{self.prefix}:idx:type:{note_data['type']}", note_id)
        if note_data.get('session_id'):
            conn.srem(f"{self.prefix}:idx:session:{note_data['session_id']}", note_id)

        # Delete note hash
        conn.delete(note_key)

        return True

    def _deserialize_note(self, note_data: Dict[str, str]) -> Dict[str, Any]:
        """Convert Redis hash strings back to proper types."""
        return {
            'id': int(note_data.get('id', 0)),
            'content': note_data.get('content', ''),
            'summary': note_data.get('summary', ''),
            'tags': note_data.get('tags', ''),
            'pinned': note_data.get('pinned', '0') == '1',
            'author': note_data.get('author', ''),
            'owner': note_data.get('owner', ''),
            'type': note_data.get('type', 'general'),
            'parent_id': int(note_data['parent_id']) if note_data.get('parent_id') else None,
            'created': note_data.get('created', ''),
            'session_id': int(note_data['session_id']) if note_data.get('session_id') else None,
            'pagerank': float(note_data.get('pagerank', 0.01)),
            'has_vector': note_data.get('has_vector', '0') == '1',
            'linked_items': json.loads(note_data.get('linked_items', '[]'))
        }

    # ==================== EDGES ====================

    def add_edge(self, from_id: int, to_id: int, edge_type: str, weight: float = 1.0) -> None:
        """Add a graph edge between notes."""
        conn = self._get_conn()
        created = datetime.utcnow().isoformat()

        # Forward edge
        edge_key = f"{self.prefix}:edges:{from_id}"
        edge_value = f"{weight}|{created}"
        conn.hset(edge_key, f"{to_id}:{edge_type}", edge_value)

        # Reverse edge (for backlinks)
        reverse_key = f"{self.prefix}:edges:reverse:{to_id}"
        conn.hset(reverse_key, f"{from_id}:{edge_type}", edge_value)

    def get_edges(self, note_id: int, reverse: bool = False) -> List[Dict[str, Any]]:
        """Get edges from a note (or to a note if reverse=True)."""
        conn = self._get_conn()

        if reverse:
            edge_key = f"{self.prefix}:edges:reverse:{note_id}"
        else:
            edge_key = f"{self.prefix}:edges:{note_id}"

        edges_data = conn.hgetall(edge_key)

        edges = []
        for key, value in edges_data.items():
            to_id_str, edge_type = key.rsplit(':', 1)
            to_id = int(to_id_str)
            weight_str, created = value.split('|')

            edges.append({
                'from_id': note_id if not reverse else to_id,
                'to_id': to_id if not reverse else note_id,
                'type': edge_type,
                'weight': float(weight_str),
                'created': created
            })

        return edges

    # ==================== VAULT ====================

    def vault_set(self, key: str, encrypted_value: bytes, author: str) -> None:
        """Store encrypted value in vault."""
        conn = self._get_conn()
        vault_key = f"{self.prefix}:vault:{key}"

        created = datetime.utcnow().isoformat()
        vault_data = {
            'encrypted_value': encrypted_value if isinstance(encrypted_value, str) else encrypted_value.decode('latin1'),
            'created': created,
            'updated': created,
            'author': author
        }

        try:
            # Try modern syntax first (Redis 4.0+)
            conn.hset(vault_key, mapping=vault_data)
        except redis.exceptions.ResponseError:
            # Fallback to hmset for older Redis (3.x)
            conn.hmset(vault_key, vault_data)

    def vault_get(self, key: str) -> Optional[bytes]:
        """Retrieve encrypted value from vault."""
        conn = self._get_conn()
        vault_key = f"{self.prefix}:vault:{key}"
        vault_data = conn.hgetall(vault_key)

        if not vault_data:
            return None

        value = vault_data.get('encrypted_value')
        # Convert string back to bytes
        return value.encode('latin1') if value else None

    def vault_delete(self, key: str) -> bool:
        """Delete value from vault."""
        conn = self._get_conn()
        vault_key = f"{self.prefix}:vault:{key}"
        return conn.delete(vault_key) > 0

    def vault_list(self) -> List[Dict[str, Any]]:
        """List all vault keys with metadata."""
        conn = self._get_conn()

        # Security: Use SCAN instead of KEYS to avoid blocking Redis server (DoS prevention)
        vault_pattern = f"{self.prefix}:vault:*"
        vault_keys = []
        cursor = 0
        while True:
            cursor, keys = conn.scan(cursor, match=vault_pattern, count=100)
            vault_keys.extend(keys)
            if cursor == 0:
                break

        items = []
        for vault_key in vault_keys:
            # Extract the actual key name (remove prefix)
            key_name = vault_key.replace(f"{self.prefix}:vault:", "")

            # Get vault data to retrieve updated timestamp
            vault_data = conn.hgetall(vault_key)
            if vault_data and 'updated' in vault_data:
                items.append({
                    'key': key_name,
                    'updated': vault_data['updated']
                })

        # Sort by updated desc (most recent first)
        items.sort(key=lambda x: x.get('updated', ''), reverse=True)
        return items

    # ==================== SESSIONS ====================

    def create_session(self) -> int:
        """Create a new session."""
        conn = self._get_conn()
        return conn.incr(f"{self.prefix}:seq:sessions")

    # ==================== STATS ====================

    def get_stats(self) -> Dict[str, int]:
        """Get teambook statistics."""
        conn = self._get_conn()

        return {
            'total_notes': conn.zcard(f"{self.prefix}:notes:timeline"),
            'pinned_notes': conn.scard(f"{self.prefix}:notes:pinned"),
            'next_note_id': int(conn.get(f"{self.prefix}:seq:notes") or 0),
            'next_session_id': int(conn.get(f"{self.prefix}:seq:sessions") or 0)
        }
