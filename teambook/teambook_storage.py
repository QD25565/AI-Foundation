#!/usr/bin/env python3
"""
TEAMBOOK MCP v1.0.0 - DATA PERSISTENCE LAYER
=====================================================
All database operations, vector storage, and persistence for teambook.
This layer handles DuckDB, ChromaDB, vault encryption, and PageRank calculations.

Built by AIs, for AIs.
=====================================================
"""

import os
import sys
import time
import json
import logging
import hashlib
from datetime import datetime, timezone
from typing import Optional, List, Tuple, Any, Dict
from pathlib import Path
from cryptography.fernet import Fernet

# Database engine
try:
    import duckdb
except ImportError:
    print("FATAL: DuckDB not installed. Please run 'pip install duckdb'", file=sys.stderr)
    sys.exit(1)

# Vector DB and embeddings
try:
    import chromadb
    from chromadb.config import Settings
    CHROMADB_AVAILABLE = True
except ImportError:
    CHROMADB_AVAILABLE = False
    logging.warning("ChromaDB not installed - semantic features disabled")

try:
    from sentence_transformers import SentenceTransformer
    ST_AVAILABLE = True
except ImportError:
    ST_AVAILABLE = False
    logging.warning("sentence-transformers not installed - semantic features disabled")

# Try to import compression utilities for storage optimization (Phase 3)
try:
    from compression_utils import compress_content, decompress_content
    COMPRESSION_AVAILABLE = True
except ImportError:
    COMPRESSION_AVAILABLE = False
    compress_content = lambda x: x.encode('utf-8') if isinstance(x, str) else x
    decompress_content = lambda x: x.decode('utf-8') if isinstance(x, bytes) else x

import numpy as np

# Import shared utilities
try:
    # Try relative import first (when imported as module)
    from .teambook_shared import (
        get_db_file, get_vector_dir, get_vault_key_file,
        KNOWN_ENTITIES, CURRENT_AI_ID, CURRENT_TEAMBOOK,
        TEMPORAL_EDGES, SESSION_GAP_MINUTES, PAGERANK_ITERATIONS,
        PAGERANK_DAMPING, USE_SEMANTIC, extract_references,
        extract_entities, logging
    )
except ImportError:
    # Fall back to absolute import (when run directly)
    from teambook_shared import (
        get_db_file, get_vector_dir, get_vault_key_file,
        KNOWN_ENTITIES, CURRENT_AI_ID, CURRENT_TEAMBOOK,
        TEMPORAL_EDGES, SESSION_GAP_MINUTES, PAGERANK_ITERATIONS,
        PAGERANK_DAMPING, USE_SEMANTIC, extract_references,
        extract_entities, logging
    )

# Redis pub/sub for real-time events
try:
    from .teambook_pubsub import publish_note_created, publish_note_updated, init_pubsub
    PUBSUB_AVAILABLE = True
except ImportError:
    try:
        from teambook_pubsub import publish_note_created, publish_note_updated, init_pubsub
        PUBSUB_AVAILABLE = True
    except ImportError:
        PUBSUB_AVAILABLE = False
        logging.debug("Redis pub/sub not available")



# ============= GLOBAL STORAGE STATE =============
encoder = None
chroma_client = None
collection = None
vault_manager = None
EMBEDDING_MODEL = None
FTS_ENABLED = False

# Connection cache - reuse connections to reduce lock contention
_connection_cache = None
_connection_cache_time = 0
_CONNECTION_CACHE_TTL = 5.0  # Reuse connection for 5 seconds


def _serialize_for_hash(payload: Dict[str, Any]) -> str:
    def _default(obj: Any) -> Any:
        if isinstance(obj, datetime):
            return obj.isoformat()
        if isinstance(obj, (set, tuple)):
            return list(obj)
        return obj

    return json.dumps(payload, sort_keys=True, default=_default)


def compute_note_tamper_hash(payload: Dict[str, Any]) -> str:
    serial = _serialize_for_hash({
        'content': payload.get('content'),
        'summary': payload.get('summary'),
        'tags': payload.get('tags') or [],
        'pinned': bool(payload.get('pinned')),
        'owner': payload.get('owner'),
        'teambook_name': payload.get('teambook_name'),
        'linked_items': payload.get('linked_items'),
        'representation_policy': payload.get('representation_policy', 'default'),
        'metadata': payload.get('metadata'),
        'type': payload.get('type'),
        'parent_id': payload.get('parent_id'),
    })
    return hashlib.sha256(serial.encode('utf-8')).hexdigest()


def _should_compress(representation_policy: Optional[str]) -> bool:
    if representation_policy and str(representation_policy).lower() == 'verbatim':
        return False
    return True


def _prepare_content_for_storage(content: str, representation_policy: Optional[str]):
    compressed = None
    if content is not None and COMPRESSION_AVAILABLE and _should_compress(representation_policy):
        compressed = compress_content(content)
    return content, compressed

# ============= VAULT MANAGER =============
class VaultManager:
    """Secure encrypted storage for secrets"""
    def __init__(self):
        self.key = self._load_or_create_key()
        self.fernet = Fernet(self.key) if self.key else None
    
    def _load_or_create_key(self) -> bytes:
        vault_file = get_vault_key_file()

        # Security: Validate vault file path to prevent path traversal
        try:
            vault_file_resolved = vault_file.resolve()
            # Ensure it's in a safe directory
            parent_dir = vault_file_resolved.parent
            if '..' in str(vault_file) or '~' in str(vault_file):
                raise ValueError("Invalid vault file path")
        except Exception as e:
            logging.error(f"Vault path validation failed: {e}")
            raise

        # Security: Fix race condition with atomic file operations
        try:
            # Try to open existing file first (atomic)
            with open(vault_file, 'rb') as f:
                return f.read()
        except FileNotFoundError:
            # File doesn't exist, create it atomically
            key = Fernet.generate_key()

            # Use os.open with exclusive creation to prevent race condition
            import stat
            try:
                fd = os.open(vault_file, os.O_CREAT | os.O_EXCL | os.O_WRONLY, stat.S_IRUSR | stat.S_IWUSR)
                try:
                    os.write(fd, key)
                finally:
                    os.close(fd)
            except FileExistsError:
                # Another process created it, read it
                with open(vault_file, 'rb') as f:
                    return f.read()
            except Exception as e:
                logging.error(f"Failed to create secure vault file: {e}")
                # Clean up on failure
                if os.path.exists(vault_file):
                    try:
                        os.remove(vault_file)
                    except:
                        pass
                raise

            return key
    
    def encrypt(self, value: str) -> bytes:
        if not self.fernet:
            self.key = self._load_or_create_key()
            self.fernet = Fernet(self.key)
        return self.fernet.encrypt(value.encode())
    
    def decrypt(self, encrypted: bytes) -> str:
        if not self.fernet:
            self.key = self._load_or_create_key()
            self.fernet = Fernet(self.key)
        return self.fernet.decrypt(encrypted).decode()

# ============= DATABASE CONNECTION =============
# Track temporary databases for cleanup
_temp_databases = set()

def cleanup_temp_databases():
    """Remove temporary databases created during lock contention"""
    for temp_db in list(_temp_databases):
        try:
            if os.path.exists(temp_db):
                os.remove(temp_db)
                _temp_databases.remove(temp_db)
                logging.info(f"Cleaned up temporary database: {temp_db}")
        except Exception as e:
            logging.debug(f"Could not remove temp DB {temp_db}: {e}")

def _get_db_conn() -> duckdb.DuckDBPyConnection:
    """Returns a pooled connection to the DuckDB database (Phase 1 optimization)"""
    # Try to use connection pooling if available (Phase 1 optimization)
    try:
        from performance_utils import get_pooled_connection
        db_path = str(get_db_file())
        return get_pooled_connection(db_path)
    except ImportError:
        # Fallback to legacy caching if performance_utils not available
        pass

    global _connection_cache, _connection_cache_time

    # Check if cached connection is still valid
    current_time = time.time()
    if _connection_cache is not None and (current_time - _connection_cache_time) < _CONNECTION_CACHE_TTL:
        try:
            # Test if connection is still alive
            _connection_cache.execute("SELECT 1")
            logging.debug("Reusing cached database connection")
            return _connection_cache
        except:
            # Connection dead, clear cache
            _connection_cache = None

    db_path = str(get_db_file())

    # Optimized retry strategy: exponential backoff with jitter
    max_retries = 5
    base_delay = 0.1  # Start with 100ms

    for attempt in range(max_retries):
        try:
            conn = duckdb.connect(database=db_path, read_only=False)
            # Cache the connection
            _connection_cache = conn
            _connection_cache_time = current_time
            if attempt > 0:
                logging.debug(f"Database connection acquired on retry {attempt + 1}")
            return conn
        except duckdb.IOException as e:
            if "being used by another process" in str(e):
                if attempt < max_retries - 1:
                    # Exponential backoff with jitter: 100ms, 200ms, 400ms, 800ms
                    delay = base_delay * (2 ** attempt) + (time.time() % 0.05)  # Add jitter
                    logging.debug(f"Database locked, retry {attempt + 1}/{max_retries} after {delay:.2f}s...")
                    time.sleep(delay)
                else:
                    # Final attempt failed - create temp DB
                    temp_db = db_path.replace('.duckdb', f'_temp_{int(time.time())}.duckdb')
                    _temp_databases.add(temp_db)
                    logging.warning(f"Database lock persisted after {max_retries} retries, using temporary database: {temp_db}")

                    # Register cleanup on first temp DB creation
                    if len(_temp_databases) == 1:
                        import atexit
                        atexit.register(cleanup_temp_databases)

                    # Create connection and initialize schema for temp DB
                    conn = duckdb.connect(database=temp_db, read_only=False)
                    create_duckdb_schema(conn)
                    return conn
            else:
                raise

    # Should never reach here, but safety fallback
    raise RuntimeError("Failed to acquire database connection")

# ============= DATABASE SCHEMA =============
def create_duckdb_schema(conn: duckdb.DuckDBPyConnection):
    """Create all tables and indices for DuckDB"""
    
    conn.execute("CREATE SEQUENCE IF NOT EXISTS notes_id_seq START 1")
    
    # Main notes table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS notes (
            id BIGINT PRIMARY KEY,
            content TEXT,
            content_compressed BLOB,
            summary TEXT,
            tags VARCHAR[],
            pinned BOOLEAN DEFAULT FALSE,
            author VARCHAR NOT NULL,
            owner VARCHAR,
            teambook_name VARCHAR,
            type VARCHAR,
            parent_id BIGINT,
            created TIMESTAMPTZ NOT NULL,
            session_id BIGINT,
            linked_items TEXT,
            representation_policy VARCHAR DEFAULT 'default',
            pagerank DOUBLE DEFAULT 0.0,
            has_vector BOOLEAN DEFAULT FALSE,
            metadata TEXT,
            tamper_hash VARCHAR
        )
    ''')
    
    # Edges table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS edges (
            from_id BIGINT NOT NULL,
            to_id BIGINT NOT NULL,
            type VARCHAR NOT NULL,
            weight DOUBLE DEFAULT 1.0,
            created TIMESTAMPTZ NOT NULL,
            PRIMARY KEY(from_id, to_id, type)
        )
    ''')
    
    # Evolution outputs table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS evolution_outputs (
            id BIGINT PRIMARY KEY,
            evolution_id BIGINT NOT NULL,
            output_path TEXT NOT NULL,
            created TIMESTAMPTZ NOT NULL,
            author VARCHAR NOT NULL
        )
    ''')
    
    # Teambooks registry
    conn.execute('''
        CREATE TABLE IF NOT EXISTS teambooks (
            name VARCHAR PRIMARY KEY,
            created TIMESTAMPTZ NOT NULL,
            created_by VARCHAR NOT NULL,
            last_active TIMESTAMPTZ
        )
    ''')
    
    # Other tables
    conn.execute('''
        CREATE TABLE IF NOT EXISTS entities (
            id BIGINT PRIMARY KEY,
            name VARCHAR UNIQUE NOT NULL,
            type VARCHAR NOT NULL,
            first_seen TIMESTAMPTZ NOT NULL,
            last_seen TIMESTAMPTZ NOT NULL,
            mention_count INTEGER DEFAULT 1
        )
    ''')
    
    conn.execute('''
        CREATE TABLE IF NOT EXISTS entity_notes (
            entity_id BIGINT NOT NULL,
            note_id BIGINT NOT NULL,
            PRIMARY KEY(entity_id, note_id)
        )
    ''')
    
    conn.execute('''
        CREATE TABLE IF NOT EXISTS sessions (
            id BIGINT PRIMARY KEY,
            started TIMESTAMPTZ NOT NULL,
            ended TIMESTAMPTZ NOT NULL,
            note_count INTEGER DEFAULT 1,
            coherence_score DOUBLE DEFAULT 1.0
        )
    ''')
    
    conn.execute('''
        CREATE TABLE IF NOT EXISTS vault (
            key VARCHAR PRIMARY KEY,
            encrypted_value BLOB NOT NULL,
            created TIMESTAMPTZ NOT NULL,
            updated TIMESTAMPTZ NOT NULL,
            author VARCHAR NOT NULL
        )
    ''')
    
    conn.execute('''
        CREATE TABLE IF NOT EXISTS stats (
            id BIGINT PRIMARY KEY,
            operation VARCHAR NOT NULL,
            ts TIMESTAMPTZ NOT NULL,
            dur_ms INTEGER,
            author VARCHAR
        )
    ''')
    
    # Create indices
    indices = [
        "CREATE INDEX IF NOT EXISTS idx_notes_created ON notes(created DESC)",
        "CREATE INDEX IF NOT EXISTS idx_notes_pinned ON notes(pinned DESC, created DESC)",
        "CREATE INDEX IF NOT EXISTS idx_notes_pagerank ON notes(pagerank DESC)",
        "CREATE INDEX IF NOT EXISTS idx_notes_owner ON notes(owner)",
        "CREATE INDEX IF NOT EXISTS idx_notes_type ON notes(type)",
        "CREATE INDEX IF NOT EXISTS idx_notes_parent ON notes(parent_id)",
        "CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id)",
        "CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id)"
    ]
    
    for idx in indices:
        conn.execute(idx)
    
    # Try to set up FTS with correct DuckDB syntax
    global FTS_ENABLED
    FTS_ENABLED = False
    
    try:
        conn.execute("INSTALL fts")
        conn.execute("LOAD fts")
        # Create FTS index - this creates a virtual table fts_main_notes
        conn.execute("PRAGMA create_fts_index('notes', 'id', 'content', 'summary')")
        # Test that it works
        conn.execute("SELECT COUNT(*) FROM fts_main_notes WHERE fts_main_notes MATCH 'test'").fetchone()
        FTS_ENABLED = True
        logging.info("DuckDB FTS extension loaded and configured")
    except Exception as e1:
        if "already exists" in str(e1):
            # FTS index already exists, just test if it works
            try:
                conn.execute("LOAD fts")
                # Test FTS works
                conn.execute("SELECT COUNT(*) FROM fts_main_notes WHERE fts_main_notes MATCH 'test'").fetchone()
                FTS_ENABLED = True
                logging.info("DuckDB FTS already configured")
            except Exception as e2:
                logging.warning(f"FTS index exists but not working: {e2}")
        else:
            # FTS not available for other reasons
            logging.info(f"FTS not available, will use LIKE queries (this is fine): {str(e1)[:100]}")

def _init_db():
    """Initialize DuckDB database"""
    with _get_db_conn() as conn:
        tables = conn.execute("SHOW TABLES").fetchall()
        if not any(t[0] == 'notes' for t in tables):
            logging.info("Creating new database schema...")
            create_duckdb_schema(conn)
        else:
            # Check for missing columns and add them
            cursor = conn.execute("PRAGMA table_info(notes)")
            columns = [col[1] for col in cursor.fetchall()]
            
            migrations = [
                ('owner', 'ALTER TABLE notes ADD COLUMN owner VARCHAR'),
                ('teambook_name', 'ALTER TABLE notes ADD COLUMN teambook_name VARCHAR'),
                ('type', 'ALTER TABLE notes ADD COLUMN type VARCHAR'),
                ('parent_id', 'ALTER TABLE notes ADD COLUMN parent_id BIGINT'),
                ('content_compressed', 'ALTER TABLE notes ADD COLUMN content_compressed BLOB'),
                ('representation_policy', "ALTER TABLE notes ADD COLUMN representation_policy VARCHAR DEFAULT 'default'"),
                ('metadata', 'ALTER TABLE notes ADD COLUMN metadata TEXT'),
                ('tamper_hash', 'ALTER TABLE notes ADD COLUMN tamper_hash VARCHAR')
            ]
            
            for col_name, sql in migrations:
                if col_name not in columns:
                    logging.info(f"Adding {col_name} column...")
                    conn.execute(sql)
            
            # Ensure all tables exist
            if not any(t[0] == 'evolution_outputs' for t in tables):
                conn.execute('''
                    CREATE TABLE IF NOT EXISTS evolution_outputs (
                        id BIGINT PRIMARY KEY,
                        evolution_id BIGINT NOT NULL,
                        output_path TEXT NOT NULL,
                        created TIMESTAMPTZ NOT NULL,
                        author VARCHAR NOT NULL
                    )
                ''')
            
            if not any(t[0] == 'teambooks' for t in tables):
                conn.execute('''
                    CREATE TABLE IF NOT EXISTS teambooks (
                        name VARCHAR PRIMARY KEY,
                        created TIMESTAMPTZ NOT NULL,
                        created_by VARCHAR NOT NULL,
                        last_active TIMESTAMPTZ
                    )
                ''')
            
            # Try to initialize FTS again if not enabled
            global FTS_ENABLED
            if not FTS_ENABLED:
                try:
                    conn.execute("LOAD fts")
                    # Test if FTS works
                    conn.execute("SELECT COUNT(*) FROM fts_main_notes WHERE fts_main_notes MATCH 'test'").fetchone()
                    FTS_ENABLED = True
                    logging.info("FTS enabled on existing database")
                except Exception as e:
                    # Try to create the index if it doesn't exist
                    try:
                        conn.execute("INSTALL fts")
                        conn.execute("LOAD fts")
                        conn.execute("PRAGMA create_fts_index('notes', 'id', 'content', 'summary')")
                        # Test FTS works
                        conn.execute("SELECT COUNT(*) FROM fts_main_notes WHERE fts_main_notes MATCH 'test'").fetchone()
                        FTS_ENABLED = True
                        logging.info("FTS index created and enabled")
                    except:
                        # FTS not available, that's OK
                        pass
        
        load_known_entities(conn)

        note_count = conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
        logging.info(f"Database ready with {note_count} notes")

    # Auto-discovery: Auto-connect to Town Hall on first run
    try:
        from teambook_api import auto_connect_town_hall
        auto_connect_town_hall()
    except Exception as e:
        logging.debug(f"Town Hall auto-connect skipped: {e}")

def load_known_entities(conn: duckdb.DuckDBPyConnection):
    """Load known entities into memory cache"""
    global KNOWN_ENTITIES
    try:
        entities = conn.execute('SELECT name FROM entities').fetchall()
        KNOWN_ENTITIES.clear()
        KNOWN_ENTITIES.update(e[0].lower() for e in entities)
    except:
        pass

# ============= EMBEDDING AND VECTOR DB =============
def init_embedding_model():
    """Initialize embedding model with automatic local model detection"""
    global encoder, EMBEDDING_MODEL
    
    if not ST_AVAILABLE or not USE_SEMANTIC:
        logging.info("Semantic search disabled")
        return None
    
    # First, try to find local models
    search_paths = [
        Path.cwd() / "models",  # Current dir
        Path.cwd().parent / "models",  # Parent dir  
        Path(__file__).parent / "models" if '__file__' in globals() else None,  # Script location
    ]
    
    for path in search_paths:
        if not path or not path.exists():
            continue
            
        # Look for EmbeddingGemma specifically
        embeddinggemma_path = path / "embeddinggemma-300m"
        if embeddinggemma_path.exists():
            # Check if it has the required files
            required = ["config.json", "model.safetensors", "tokenizer.json"]
            if all((embeddinggemma_path / f).exists() for f in required):
                try:
                    logging.info(f"Loading EmbeddingGemma 300m from {embeddinggemma_path}...")
                    encoder = SentenceTransformer(str(embeddinggemma_path), device='cpu')
                    test = encoder.encode("test", convert_to_numpy=True)
                    EMBEDDING_MODEL = 'embeddinggemma-300m'
                    logging.info(f"✅ Using local EmbeddingGemma 300m (dim: {test.shape[0]})")
                    return encoder
                except Exception as e:
                    logging.warning(f"Failed to load EmbeddingGemma: {e}")
    
    # Fallback to downloading models
    logging.info("No local models found, downloading online models...")
    
    try:
        models = [
            ('sentence-transformers/all-MiniLM-L6-v2', 'minilm'),
            ('BAAI/bge-base-en-v1.5', 'bge-base'),
        ]
        
        for model_name, short_name in models:
            try:
                logging.info(f"Loading {model_name}...")
                encoder = SentenceTransformer(model_name, device='cpu')
                test = encoder.encode("test", convert_to_numpy=True)
                EMBEDDING_MODEL = short_name
                logging.info(f"✓ Using {short_name} (dim: {test.shape[0]})")
                return encoder
            except Exception as e:
                logging.debug(f"Failed to load {model_name}: {e}")
        
        logging.error("No embedding model could be loaded")
        return None
    except Exception as e:
        logging.error(f"Failed to initialize embeddings: {e}")
        return None

def _init_vector_db():
    """Initialize ChromaDB for vector storage"""
    global chroma_client, collection
    if not CHROMADB_AVAILABLE or not encoder:
        return False
    
    try:
        vector_dir = get_vector_dir()
        vector_dir.mkdir(parents=True, exist_ok=True)
        
        chroma_client = chromadb.PersistentClient(
            path=str(vector_dir),
            settings=Settings(anonymized_telemetry=False, allow_reset=True)
        )
        
        collection_name = f"teambook_{CURRENT_TEAMBOOK}_v7" if CURRENT_TEAMBOOK else "teambook_private_v7"
        collection = chroma_client.get_or_create_collection(
            name=collection_name,
            metadata={"hnsw:space": "cosine"}
        )
        
        logging.info(f"ChromaDB initialized with {collection.count()} vectors for {collection_name}")
        return True
    except Exception as e:
        logging.error(f"ChromaDB init failed: {e}")
        return False

def _init_vault_manager():
    """Initialize or reinitialize vault manager for current teambook"""
    global vault_manager
    vault_manager = VaultManager()

# ============= SESSION MANAGEMENT =============
def _detect_or_create_session(note_id: int, created: datetime, conn: duckdb.DuckDBPyConnection) -> Optional[int]:
    """Detect existing session or create new one"""
    try:
        prev = conn.execute(
            'SELECT created, session_id FROM notes WHERE id < ? ORDER BY id DESC LIMIT 1',
            [note_id]
        ).fetchone()
        
        if prev:
            prev_time = prev[0] if isinstance(prev[0], datetime) else datetime.fromisoformat(prev[0])
            if (created - prev_time).total_seconds() / 60 <= SESSION_GAP_MINUTES and prev[1]:
                conn.execute(
                    'UPDATE sessions SET ended = ?, note_count = note_count + 1 WHERE id = ?',
                    [created, prev[1]]
                )
                return prev[1]
        
        max_session_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM sessions").fetchone()[0]
        new_session_id = max_session_id + 1
        
        conn.execute(
            'INSERT INTO sessions (id, started, ended) VALUES (?, ?, ?)',
            [new_session_id, created, created]
        )
        return new_session_id
    except:
        return None

# ============= EDGE CREATION =============
def _create_all_edges(note_id: int, content: str, session_id: Optional[int], conn: duckdb.DuckDBPyConnection):
    """Create all edge types efficiently"""
    now = datetime.now(timezone.utc)
    edges_to_add = []
    
    # Temporal edges
    prev_notes = conn.execute(
        'SELECT id FROM notes WHERE id < ? ORDER BY id DESC LIMIT ?',
        [note_id, TEMPORAL_EDGES]
    ).fetchall()
    
    for prev in prev_notes:
        edges_to_add.extend([
            (note_id, prev[0], 'temporal', 1.0, now),
            (prev[0], note_id, 'temporal', 1.0, now)
        ])
    
    # Reference edges
    refs = extract_references(content)
    if refs:
        # Security: placeholders generated from list length (safe)
        placeholders = ','.join(['?'] * len(refs))
        valid_refs = conn.execute(
            f'SELECT id FROM notes WHERE id IN ({placeholders})',
            refs
        ).fetchall()
        
        for ref_id in valid_refs:
            edges_to_add.extend([
                (note_id, ref_id[0], 'reference', 2.0, now),
                (ref_id[0], note_id, 'referenced_by', 2.0, now)
            ])
    
    # Session edges
    if session_id:
        session_notes = conn.execute(
            'SELECT id FROM notes WHERE session_id = ? AND id != ?',
            [session_id, note_id]
        ).fetchall()
        
        for other in session_notes:
            edges_to_add.extend([
                (note_id, other[0], 'session', 1.5, now),
                (other[0], note_id, 'session', 1.5, now)
            ])
    
    # Entity edges (optimized - batch lookups first, then batch operations)
    entities = extract_entities(content)
    if entities:
        # Batch 1: Lookup all entities at once
        entity_names = [name for name, _ in entities]
        placeholders = ','.join(['?'] * len(entity_names))
        existing_entities = conn.execute(
            f'SELECT id, name FROM entities WHERE name IN ({placeholders})',
            entity_names
        ).fetchall()
        existing_map = {name: eid for eid, name in existing_entities}

        # Batch 2: Prepare updates and inserts
        entities_to_update = []
        entities_to_insert = []
        entity_notes_to_insert = []

        max_entity_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM entities").fetchone()[0]
        next_id = max_entity_id + 1

        for entity_name, entity_type in entities:
            if entity_name in existing_map:
                # Existing entity - prepare update
                entity_id = existing_map[entity_name]
                entities_to_update.append((now, entity_id))
            else:
                # New entity - prepare insert
                entity_id = next_id
                next_id += 1
                entities_to_insert.append((entity_id, entity_name, entity_type, now, now))
                existing_map[entity_name] = entity_id
                KNOWN_ENTITIES.add(entity_name.lower())

            # Prepare entity_notes link
            entity_notes_to_insert.append((entity_id, note_id))

        # Batch 3: Execute all updates at once
        if entities_to_update:
            conn.executemany(
                'UPDATE entities SET last_seen = ?, mention_count = mention_count + 1 WHERE id = ?',
                entities_to_update
            )

        # Batch 4: Execute all inserts at once
        if entities_to_insert:
            conn.executemany(
                'INSERT INTO entities (id, name, type, first_seen, last_seen) VALUES (?, ?, ?, ?, ?)',
                entities_to_insert
            )

        # Batch 5: Link all entity_notes at once
        if entity_notes_to_insert:
            conn.executemany(
                'INSERT INTO entity_notes (entity_id, note_id) VALUES (?, ?) ON CONFLICT DO NOTHING',
                entity_notes_to_insert
            )

        # Batch 6: Get all related notes for entity edges (OPTIMIZED - single query for all entities)
        if existing_map:
            entity_ids = list(existing_map.values())
            placeholders = ','.join(['?'] * len(entity_ids))
            # Single query to get ALL related notes for ALL entities
            all_related_notes = conn.execute(
                f'SELECT entity_id, note_id FROM entity_notes WHERE entity_id IN ({placeholders}) AND note_id != ?',
                entity_ids + [note_id]
            ).fetchall()

            # Group by entity_id and create edges
            for entity_id, other_note_id in all_related_notes:
                edges_to_add.extend([
                    (note_id, other_note_id, 'entity', 1.2, now),
                    (other_note_id, note_id, 'entity', 1.2, now)
                ])
    
    # Batch insert edges (optimized - single executemany instead of loop)
    if edges_to_add:
        conn.executemany(
            'INSERT INTO edges (from_id, to_id, type, weight, created) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING',
            edges_to_add
        )

# ============= PAGERANK CALCULATION =============
def calculate_pagerank_duckdb(conn: duckdb.DuckDBPyConnection):
    """Calculate PageRank using DuckDB's native SQL"""
    try:
        start = time.time()
        
        conn.execute(f'''
            CREATE OR REPLACE TEMPORARY TABLE pagerank_scores AS
            WITH RECURSIVE
            nodes AS (
                SELECT DISTINCT id FROM notes
            ),
            node_count AS (
                SELECT COUNT(*)::DOUBLE as total FROM nodes
            ),
            outlinks AS (
                SELECT from_id, SUM(weight) as total_weight
                FROM edges
                GROUP BY from_id
            ),
            pagerank(iteration, id, rank) AS (
                SELECT 0, id, 1.0 / node_count.total
                FROM nodes, node_count
                
                UNION ALL
                
                SELECT 
                    pr.iteration + 1,
                    n.id,
                    (1 - {PAGERANK_DAMPING}) / nc.total + 
                    {PAGERANK_DAMPING} * COALESCE(SUM(pr.rank * e.weight / ol.total_weight), 0)
                FROM nodes n
                CROSS JOIN node_count nc
                LEFT JOIN edges e ON e.to_id = n.id
                LEFT JOIN pagerank pr ON pr.id = e.from_id AND pr.iteration < {PAGERANK_ITERATIONS}
                LEFT JOIN outlinks ol ON ol.from_id = e.from_id
                WHERE pr.iteration < {PAGERANK_ITERATIONS}
                GROUP BY pr.iteration, n.id, nc.total
            )
            SELECT id, rank 
            FROM pagerank 
            WHERE iteration = {PAGERANK_ITERATIONS}
        ''')
        
        conn.execute('''
            UPDATE notes 
            SET pagerank = pr.rank
            FROM pagerank_scores pr
            WHERE notes.id = pr.id
        ''')
        
        elapsed = time.time() - start
        note_count = conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
        logging.info(f"PageRank calculated for {note_count} notes in {elapsed:.2f}s")
        
    except Exception as e:
        logging.error(f"DuckDB PageRank failed, using fallback: {e}")
        conn.execute('''
            UPDATE notes 
            SET pagerank = COALESCE((
                SELECT COUNT(*) * 0.01 
                FROM edges 
                WHERE edges.to_id = notes.id
            ), 0.01)
        ''')

def _calculate_pagerank_if_needed(conn: duckdb.DuckDBPyConnection):
    """Calculate PageRank when needed"""
    from teambook_shared import PAGERANK_DIRTY, PAGERANK_CACHE_TIME, PAGERANK_CACHE_SECONDS
    
    count = conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
    if count < 50:
        return
    
    current_time = time.time()
    if PAGERANK_DIRTY or (current_time - PAGERANK_CACHE_TIME > PAGERANK_CACHE_SECONDS):
        calculate_pagerank_duckdb(conn)
        # Update shared state
        import teambook_shared
        teambook_shared.PAGERANK_DIRTY = False
        teambook_shared.PAGERANK_CACHE_TIME = current_time

# ============= STATS TRACKING =============
def _log_operation_to_db(op: str, dur_ms: int = None):
    """Log operation to database"""
    try:
        with _get_db_conn() as conn:
            max_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM stats").fetchone()[0]
            conn.execute(
                'INSERT INTO stats (id, operation, ts, dur_ms, author) VALUES (?, ?, ?, ?, ?)',
                [max_id + 1, op, datetime.now(timezone.utc), dur_ms, CURRENT_AI_ID]
            )
    except:
        pass

# ============= NOTE ID RESOLUTION =============
def _resolve_note_id(id_param: Any) -> Optional[int]:
    """
    Resolve note ID including database lookup for 'last'.

    Security: Uses centralized get_note_id() to ensure consistent ID handling
    across all API functions, preventing type confusion bugs.

    Args:
        id_param: ID in various formats (int, str, 'last', 'evo:123', etc.)

    Returns:
        Integer ID or None if invalid/not found
    """
    from teambook_shared import get_note_id, get_last_operation

    if id_param == "last":
        last_op = get_last_operation()
        if last_op and last_op['type'] in ['remember', 'write']:
            return last_op['result'].get('id')

        # Database lookup for most recent
        with _get_db_conn() as conn:
            recent = conn.execute('SELECT id FROM notes ORDER BY created DESC LIMIT 1').fetchone()
            return recent[0] if recent else None

    # Security: Use centralized ID normalization
    return get_note_id(id_param)

# ============= VECTOR OPERATIONS =============
def _add_to_vector_store(note_id: int, content: str, summary: str, tags: List[str]):
    """Add a note to the vector store"""
    if not encoder or not collection:
        return False
    
    try:
        embedding = encoder.encode(content[:1000], convert_to_numpy=True)
        collection.add(
            embeddings=[embedding.tolist()],
            documents=[content],
            metadatas={
                "created": datetime.now(timezone.utc).isoformat(),
                "summary": summary,
                "tags": json.dumps(tags)
            },
            ids=[str(note_id)]
        )
        return True
    except Exception as e:
        logging.warning(f"Vector storage failed: {e}")
        return False

def _search_vectors(query: str, limit: int = 100) -> List[int]:
    """Search vector store for similar content"""
    if not encoder or not collection:
        return []

    try:
        query_embedding = encoder.encode(query, convert_to_numpy=True)
        results = collection.query(
            query_embeddings=[query_embedding.tolist()],
            n_results=min(limit, 100)
        )
        if results['ids'] and results['ids'][0]:
            return [int(id_str) for id_str in results['ids'][0]]
    except Exception as e:
        logging.debug(f"Vector search failed: {e}")

    return []


# Wrapper class for storage adapter compatibility
class DuckDBTeambookStorage:
    """
    Wrapper class that provides the same interface as PostgreSQL/Redis backends
    but uses the existing DuckDB function-based implementation.
    """

    def __init__(self, teambook_name: str):
        """Initialize DuckDB storage for given teambook"""
        global CURRENT_TEAMBOOK
        self.teambook_name = teambook_name
        # Set global CURRENT_TEAMBOOK for DuckDB operations
        CURRENT_TEAMBOOK = teambook_name
        # Ensure teambook exists
        _init_db()

    def write_note(self, content: str, summary: str = "", tags: List[str] = None,
                   pinned: bool = False, linked_items: str = None,
                   owner: str = None, note_type: str = None,
                   parent_id: int = None, representation_policy: str = 'default',
                   metadata: Any = None) -> int:
        """Write a note to DuckDB"""
        with _get_db_conn() as conn:
            max_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM notes").fetchone()[0]
            note_id = max_id + 1
            tags = tags or []
            owner = owner or CURRENT_AI_ID
            now = datetime.now(timezone.utc)

            representation_policy = (representation_policy or 'default').strip().lower()

            # Store plain content plus optional compressed cache
            stored_content, compressed_payload = _prepare_content_for_storage(content, representation_policy)
            stored_summary = summary
            if summary and COMPRESSION_AVAILABLE and _should_compress(representation_policy):
                stored_summary = compress_content(summary)

            normalized_metadata = json.dumps(metadata) if isinstance(metadata, (dict, list)) else metadata

            tamper_hash = compute_note_tamper_hash({
                'content': content,
                'summary': summary,
                'tags': tags,
                'pinned': pinned,
                'owner': owner,
                'teambook_name': self.teambook_name,
                'linked_items': linked_items,
                'representation_policy': representation_policy,
                'metadata': normalized_metadata,
                'type': note_type,
                'parent_id': parent_id,
            })

            conn.execute('''
                INSERT INTO notes (
                    id, content, content_compressed, summary, tags, pinned, author, owner,
                    teambook_name, type, parent_id, created, session_id, linked_items,
                    representation_policy, pagerank, has_vector, metadata, tamper_hash
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ''', [
                note_id,
                stored_content,
                compressed_payload,
                stored_summary,
                tags,
                pinned,
                CURRENT_AI_ID,
                owner,
                self.teambook_name,
                note_type,
                parent_id,
                now,
                None,
                linked_items,
                representation_policy,
                0.0,
                False,
                normalized_metadata,
                tamper_hash
            ])

            return note_id

    def read_notes(self, limit: int = 20, pinned_only: bool = False,
                   owner: str = None, note_type: str = None,
                   tag: str = None, query: str = None,
                   mode: str = "recent") -> List[dict]:
        """Read notes from DuckDB"""
        with _get_db_conn() as conn:
            conditions = ["teambook_name = ?"]
            params = [self.teambook_name]

            if pinned_only:
                conditions.append("pinned = TRUE")
            if owner:
                conditions.append("owner = ?")
                params.append(owner)
            if note_type:
                conditions.append("type = ?")
                params.append(note_type)
            if tag:
                conditions.append("? = ANY(tags)")
                params.append(tag)
            if query:
                conditions.append("(content LIKE ? OR summary LIKE ?)")
                search_pattern = f"%{query}%"
                params.append(search_pattern)
                params.append(search_pattern)

            where_clause = " AND ".join(conditions)
            order = "ORDER BY pinned DESC, pagerank DESC, created DESC" if mode == "important" else "ORDER BY created DESC"

            sql = f"SELECT * FROM notes WHERE {where_clause} {order} LIMIT ?"
            params.append(limit)

            rows = conn.execute(sql, params).fetchall()
            columns = [desc[0] for desc in conn.description]
            notes = [dict(zip(columns, row)) for row in rows]

            # Decompress content and summary while preserving plaintext caches
            if COMPRESSION_AVAILABLE:
                for note in notes:
                    compressed_body = note.get('content_compressed')
                    if compressed_body:
                        note['content'] = decompress_content(compressed_body)
                    elif isinstance(note.get('content'), bytes):
                        note['content'] = decompress_content(note['content'])

                    if isinstance(note.get('summary'), bytes):
                        note['summary'] = decompress_content(note['summary'])

                    note.pop('content_compressed', None)

            return notes

    def get_note(self, note_id: int) -> Optional[dict]:
        """Get a single note by ID"""
        with _get_db_conn() as conn:
            row = conn.execute("SELECT * FROM notes WHERE id = ?", [note_id]).fetchone()
            if row:
                columns = [desc[0] for desc in conn.description]
                note = dict(zip(columns, row))

                # Decompress content and summary if needed (Phase 3)
                if COMPRESSION_AVAILABLE:
                    compressed_body = note.get('content_compressed')
                    if compressed_body:
                        note['content'] = decompress_content(compressed_body)
                    elif isinstance(note.get('content'), bytes):
                        note['content'] = decompress_content(note['content'])
                    if isinstance(note.get('summary'), bytes):
                        note['summary'] = decompress_content(note['summary'])
                    note.pop('content_compressed', None)

                return note
            return None

    def update_note(self, note_id: int, **updates) -> bool:
        """Update note fields with tamper-evident hashing."""
        if not updates:
            return False

        allowed_columns = {
            'content', 'summary', 'tags', 'pinned', 'owner', 'claimed_by',
            'assigned_to', 'status', 'type', 'parent_id', 'metadata',
            'linked_items', 'representation_policy'
        }

        filtered_updates = {k: v for k, v in updates.items() if k in allowed_columns}
        if not filtered_updates:
            return False

        with _get_db_conn() as conn:
            row = conn.execute("SELECT * FROM notes WHERE id = ?", [note_id]).fetchone()
            if not row:
                return False
            columns = [desc[0] for desc in conn.description]

        current = dict(zip(columns, row))

        # Normalize current payload for hashing/compression decisions
        if current.get('content_compressed'):
            current_content = decompress_content(current['content_compressed']) if COMPRESSION_AVAILABLE else current['content_compressed']
        elif isinstance(current.get('content'), bytes):
            current_content = decompress_content(current['content']) if COMPRESSION_AVAILABLE else current['content']
        else:
            current_content = current.get('content')

        if isinstance(current.get('summary'), bytes):
            current_summary = decompress_content(current['summary']) if COMPRESSION_AVAILABLE else current['summary']
        else:
            current_summary = current.get('summary')

        current_payload = {
            'content': current_content,
            'summary': current_summary,
            'tags': current.get('tags'),
            'pinned': current.get('pinned'),
            'owner': current.get('owner'),
            'claimed_by': current.get('claimed_by'),
            'assigned_to': current.get('assigned_to'),
            'status': current.get('status'),
            'type': current.get('type'),
            'parent_id': current.get('parent_id'),
            'metadata': current.get('metadata'),
            'linked_items': current.get('linked_items'),
            'representation_policy': current.get('representation_policy', 'default') or 'default'
        }

        # Merge updates into payload
        for key, value in filtered_updates.items():
            if key == 'metadata' and isinstance(value, (dict, list)):
                current_payload[key] = json.dumps(value)
            else:
                current_payload[key] = value

        # Sanitize representation policy
        representation_policy = (current_payload.get('representation_policy') or 'default').strip().lower()
        current_payload['representation_policy'] = representation_policy

        if 'tags' in filtered_updates and isinstance(current_payload['tags'], str):
            current_payload['tags'] = [current_payload['tags']]

        db_updates: Dict[str, Any] = {}

        # Prepare content updates when content or policy changes
        if 'content' in filtered_updates or 'representation_policy' in filtered_updates:
            stored_content, compressed_payload = _prepare_content_for_storage(current_payload.get('content'), representation_policy)
            db_updates['content'] = stored_content
            db_updates['content_compressed'] = compressed_payload

        if 'summary' in filtered_updates or 'representation_policy' in filtered_updates:
            summary_value = current_payload.get('summary')
            if summary_value and COMPRESSION_AVAILABLE and _should_compress(representation_policy):
                db_updates['summary'] = compress_content(summary_value)
            else:
                db_updates['summary'] = summary_value

        for simple_field in ['tags', 'pinned', 'owner', 'claimed_by', 'assigned_to', 'status', 'type', 'parent_id', 'linked_items', 'metadata']:
            if simple_field in filtered_updates:
                db_updates[simple_field] = current_payload.get(simple_field)

        if representation_policy != current.get('representation_policy') or 'representation_policy' in filtered_updates:
            db_updates['representation_policy'] = representation_policy

        # Recompute tamper hash using stored representations
        tamper_hash = compute_note_tamper_hash({
            'content': current_payload.get('content'),
            'summary': current_payload.get('summary'),
            'tags': current_payload.get('tags'),
            'pinned': current_payload.get('pinned'),
            'owner': current_payload.get('owner'),
            'teambook_name': current.get('teambook_name'),
            'linked_items': current_payload.get('linked_items'),
            'representation_policy': representation_policy,
            'metadata': current_payload.get('metadata'),
            'type': current_payload.get('type'),
            'parent_id': current_payload.get('parent_id'),
        })
        db_updates['tamper_hash'] = tamper_hash

        if not db_updates:
            return False

        set_clause = ", ".join(f"{col} = ?" for col in db_updates.keys())
        values = list(db_updates.values())
        values.append(note_id)

        with _get_db_conn() as conn:
            conn.execute(f"UPDATE notes SET {set_clause} WHERE id = ?", values)
        return True

    def delete_note(self, note_id: int) -> bool:
        """Delete a note"""
        with _get_db_conn() as conn:
            # Check if note exists first
            exists = conn.execute("SELECT 1 FROM notes WHERE id = ?", [note_id]).fetchone()
            if not exists:
                return False
            conn.execute("DELETE FROM notes WHERE id = ?", [note_id])
            return True

    def add_edge(self, from_id: int, to_id: int, edge_type: str, weight: float = 1.0) -> None:
        """Add a graph edge"""
        with _get_db_conn() as conn:
            now = datetime.now(timezone.utc)
            conn.execute('''
                INSERT OR REPLACE INTO edges (from_id, to_id, type, weight, created)
                VALUES (?, ?, ?, ?, ?)
            ''', [from_id, to_id, edge_type, weight, now])

    def get_edges(self, note_id: int, reverse: bool = False) -> List[dict]:
        """Get edges from/to a note"""
        with _get_db_conn() as conn:
            if reverse:
                rows = conn.execute("SELECT * FROM edges WHERE to_id = ?", [note_id]).fetchall()
            else:
                rows = conn.execute("SELECT * FROM edges WHERE from_id = ?", [note_id]).fetchall()
            columns = [desc[0] for desc in conn.description]
            return [dict(zip(columns, row)) for row in rows]

    def vault_set(self, key: str, encrypted_value: bytes, author: str) -> None:
        """Store encrypted value"""
        with _get_db_conn() as conn:
            now = datetime.now(timezone.utc)
            conn.execute('''
                INSERT OR REPLACE INTO vault (key, encrypted_value, created, updated, author)
                VALUES (?, ?, ?, ?, ?)
            ''', [key, encrypted_value, now, now, author])

    def vault_get(self, key: str) -> Optional[bytes]:
        """Retrieve encrypted value"""
        with _get_db_conn() as conn:
            row = conn.execute("SELECT encrypted_value FROM vault WHERE key = ?", [key]).fetchone()
            return row[0] if row else None

    def vault_delete(self, key: str) -> bool:
        """Delete encrypted value"""
        with _get_db_conn() as conn:
            result = conn.execute("DELETE FROM vault WHERE key = ?", [key])
            return result.rowcount > 0

    def vault_list(self) -> List[Dict[str, Any]]:
        """List all vault keys with metadata"""
        with _get_db_conn() as conn:
            rows = conn.execute('''
                SELECT key, updated
                FROM vault
                ORDER BY updated DESC
            ''').fetchall()

            items = []
            for key, updated in rows:
                items.append({
                    'key': key,
                    'updated': updated
                })
            return items

    def create_session(self) -> int:
        """Create a new session"""
        with _get_db_conn() as conn:
            now = datetime.now(timezone.utc)
            max_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM sessions").fetchone()[0]
            session_id = max_id + 1
            conn.execute('''
                INSERT INTO sessions (id, started, ended, note_count)
                VALUES (?, ?, ?, ?)
            ''', [session_id, now, now, 1])
            return session_id

    def get_stats(self) -> dict:
        """Get teambook statistics"""
        with _get_db_conn() as conn:
            total = conn.execute("SELECT COUNT(*) FROM notes WHERE teambook_name = ?", [self.teambook_name]).fetchone()[0]
            pinned = conn.execute("SELECT COUNT(*) FROM notes WHERE teambook_name = ? AND pinned = TRUE", [self.teambook_name]).fetchone()[0]
            return {
                'total_notes': total,
                'pinned_notes': pinned,
                'backend': 'duckdb'
            }




# Export aliases for backward compatibility
get_db_conn = _get_db_conn
log_operation_to_db = _log_operation_to_db
