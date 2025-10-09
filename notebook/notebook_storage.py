#!/usr/bin/env python3
"""
NOTEBOOK MCP v1.0.0 - STORAGE MODULE
=====================================
Database operations, vector storage, and persistence for the Notebook MCP tool.
Handles DuckDB, ChromaDB, and all data interactions.

v1.0.0 - First Public Release:
- Separated storage logic from main application
- Enhanced with directory tracking integration
- Fixed pinned_only query bug
=====================================
"""

import json
import sys
import os
import shutil
import time
import sqlite3
import re
from collections import deque
import numpy as np
from datetime import datetime
from pathlib import Path
from typing import Optional, Dict, List, Any, Tuple
import logging
from cryptography.fernet import Fernet

# Try to import compression utilities for storage optimization
try:
    from compression_utils import compress_content, decompress_content
    COMPRESSION_AVAILABLE = True
except ImportError:
    COMPRESSION_AVAILABLE = False
    compress_content = lambda x: x.encode('utf-8') if isinstance(x, str) else x
    decompress_content = lambda x: x.decode('utf-8') if isinstance(x, bytes) else x

# Suppress noisy third-party library logs
logging.getLogger('sentence_transformers').setLevel(logging.WARNING)
logging.getLogger('chromadb').setLevel(logging.WARNING)

# Fix import path for src/ structure
sys.path.insert(0, str(Path(__file__).parent))

# Import shared utilities
from notebook_shared import *

# Database Engine
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

# Module-level storage state
encoder = None
chroma_client = None
collection = None
EMBEDDING_MODEL = None
ACTIVE_EMBED_DIM: Optional[int] = None
FTS_ENABLED = False
_embeddings_initialized = False  # Track lazy initialization
_logged_once = set()  # Track one-time log messages to reduce noise

# Target embedding sizes allow us to standardize vectors across notebook instances.
# embeddinggemma-300m ships with 1024d outputs by default. We project down to 512d
# to shrink storage and improve retrieval latency while keeping deterministic output.
TARGET_EMBED_DIMS: Dict[str, int] = {
    "embeddinggemma-300m": 512,
}

# Lightweight fact extraction powered by deterministic regex patterns. These
# patterns capture high-signal relationship statements so we can enrich the
# temporal knowledge graph without heavy NLP dependencies. Each entry specifies
# the canonical relation name, regex strings, and whether matching a new fact
# should invalidate prior facts for the same entity/relation pair.
FACT_PATTERNS: List[Dict[str, Any]] = [
    {
        "relation": "resides_in",
        "patterns": [
            r"(?P<subject>[A-Z][\w\s]+?)\s+(?:lives in|lives at|is based in)\s+(?P<object>[A-Z][\w\s]+)",
            r"(?P<subject>[A-Z][\w\s]+?)\s+moved to\s+(?P<object>[A-Z][\w\s]+)"
        ],
        "invalidate": True,
        "confidence": 0.85,
    },
    {
        "relation": "works_at",
        "patterns": [
            r"(?P<subject>[A-Z][\w\s]+?)\s+(?:works at|works for|joined)\s+(?P<object>[A-Z][\w\s&]+)"
        ],
        "invalidate": True,
        "confidence": 0.8,
    },
    {
        "relation": "located_in",
        "patterns": [
            r"(?P<subject>[A-Z][\w\s]+?)\s+(?:is located in|is in|operates in)\s+(?P<object>[A-Z][\w\s]+)"
        ],
        "invalidate": False,
        "confidence": 0.75,
    },
]


def _project_embedding(vector: np.ndarray) -> np.ndarray:
    """Project embedding vectors to the configured target dimension (if any)."""

    global ACTIVE_EMBED_DIM

    if vector is None:
        return vector

    try:
        current_dim = vector.shape[0]
    except Exception:
        return vector

    target_dim = TARGET_EMBED_DIMS.get(EMBEDDING_MODEL or "")
    if not target_dim or current_dim == target_dim:
        ACTIVE_EMBED_DIM = current_dim
        return vector

    if current_dim > target_dim:
        # Deterministic average pooling across the extra dimensions. This preserves
        # order and keeps output stable across runs without adding heavy deps.
        if current_dim % target_dim == 0:
            step = current_dim // target_dim
            reshaped = vector.reshape(target_dim, step)
            result = reshaped.mean(axis=1)
            ACTIVE_EMBED_DIM = target_dim
            return result
        ACTIVE_EMBED_DIM = target_dim
        return vector[:target_dim]

    # Pad with zeros if the model unexpectedly returns a smaller vector.
    padded = np.zeros(target_dim, dtype=vector.dtype)
    padded[:current_dim] = vector
    ACTIVE_EMBED_DIM = target_dim
    return padded


def build_embedding_document(content: Optional[str], summary: Optional[str], tags: Optional[str]) -> str:
    """Compose the text payload stored alongside embeddings."""

    parts: List[str] = []
    if content:
        parts.append(str(content).strip())
    if summary and summary not in parts:
        parts.append(str(summary).strip())
    if tags:
        parts.append(str(tags).strip())
    document = " \n".join(part for part in parts if part)  # Keep ordering but separate
    return document[:1000]


def generate_embedding(text: str) -> Optional[np.ndarray]:
    """Encode text with the active embedding model and normalize dimensions."""

    if encoder is None:
        return None

    sanitized = (text or "").strip()
    if not sanitized:
        return None

    vector = encoder.encode(sanitized[:1000], convert_to_numpy=True)
    if isinstance(vector, list):
        vector = np.asarray(vector)
    return _project_embedding(vector)

class VaultManager:
    """Secure encrypted storage for secrets"""
    def __init__(self):
        self.key = self._load_or_create_key()
        self.fernet = Fernet(self.key)
    
    def _load_or_create_key(self) -> bytes:
        # Security: Validate vault file path to prevent path traversal
        vault_file = VAULT_KEY_FILE
        try:
            vault_file_resolved = vault_file.resolve()
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
        return self.fernet.encrypt(value.encode())
    
    def decrypt(self, encrypted: bytes) -> str:
        return self.fernet.decrypt(encrypted).decode()

vault_manager = VaultManager()

def _get_db_conn() -> duckdb.DuckDBPyConnection:
    """Returns a pooled connection to the DuckDB database (CLI and MCP compatible)"""
    # Try to use connection pooling if available
    try:
        from performance_utils import get_pooled_connection
        return get_pooled_connection(str(DB_FILE))
    except ImportError:
        # Fallback to direct connection if performance_utils not available
        return duckdb.connect(database=str(DB_FILE), read_only=False)

def _migrate_simple_table(sqlite_conn, duck_conn, table_name: str):
    """Helper to migrate a simple table from SQLite to DuckDB"""
    try:
        rows = sqlite_conn.execute(f"SELECT * FROM {table_name}").fetchall()
        if rows:
            placeholders = ','.join(['?'] * len(rows[0]))
            duck_conn.executemany(
                f"INSERT INTO {table_name} VALUES ({placeholders})",
                rows
            )
    except Exception as e:
        logging.warning(f"Could not migrate {table_name}: {e}")

def _create_duckdb_schema(conn: duckdb.DuckDBPyConnection):
    """Create all tables and indices for DuckDB"""
    
    conn.execute("CREATE SEQUENCE IF NOT EXISTS notes_id_seq START 1")
    
    # Main notes table with native array for tags
    conn.execute('''
        CREATE TABLE IF NOT EXISTS notes (
            id BIGINT PRIMARY KEY,
            content TEXT,
            summary TEXT,
            tags VARCHAR[],
            pinned BOOLEAN DEFAULT FALSE,
            author VARCHAR NOT NULL,
            created TIMESTAMPTZ NOT NULL,
            session_id BIGINT,
            linked_items TEXT,
            pagerank DOUBLE DEFAULT 0.0,
            has_vector BOOLEAN DEFAULT FALSE
        )
    ''')
    
    # Directory tracking table
    conn.execute('''
        CREATE TABLE IF NOT EXISTS directory_access (
            id BIGINT PRIMARY KEY,
            path TEXT NOT NULL,
            accessed TIMESTAMPTZ NOT NULL,
            note_id BIGINT,
            operation VARCHAR
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
            valid_from TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            valid_to TIMESTAMPTZ,
            source_note_id BIGINT,
            metadata JSON,
            PRIMARY KEY(from_id, to_id, type)
        )
    ''')

    conn.execute("CREATE SEQUENCE IF NOT EXISTS entity_facts_id_seq START 1")

    conn.execute('''
        CREATE TABLE IF NOT EXISTS entity_facts (
            id BIGINT PRIMARY KEY,
            entity_id BIGINT NOT NULL,
            relation VARCHAR NOT NULL,
            value TEXT NOT NULL,
            target_entity_id BIGINT,
            valid_from TIMESTAMPTZ NOT NULL,
            valid_to TIMESTAMPTZ,
            source_note_id BIGINT NOT NULL,
            confidence DOUBLE DEFAULT 0.7,
            metadata JSON
        )
    ''')

    conn.execute('''
        CREATE INDEX IF NOT EXISTS idx_entity_facts_lookup
        ON entity_facts(entity_id, relation)
    ''')


def _ensure_temporal_graph_schema(conn: duckdb.DuckDBPyConnection) -> None:
    """Backfill new temporal graph columns for existing databases."""

    try:
        info = conn.execute("PRAGMA table_info('edges')").fetchall()
        columns = {row[1] for row in info}

        if 'valid_from' not in columns:
            conn.execute("ALTER TABLE edges ADD COLUMN valid_from TIMESTAMPTZ")
            conn.execute("UPDATE edges SET valid_from = created WHERE valid_from IS NULL")

        if 'valid_to' not in columns:
            conn.execute("ALTER TABLE edges ADD COLUMN valid_to TIMESTAMPTZ")

        if 'source_note_id' not in columns:
            conn.execute("ALTER TABLE edges ADD COLUMN source_note_id BIGINT")

        if 'metadata' not in columns:
            conn.execute("ALTER TABLE edges ADD COLUMN metadata JSON")
    except Exception as exc:
        logging.debug(f"Temporal graph schema check skipped: {exc}")


def _ensure_entity_facts_schema(conn: duckdb.DuckDBPyConnection) -> None:
    """Ensure the entity_facts table exists for temporal knowledge graph storage."""

    conn.execute("CREATE SEQUENCE IF NOT EXISTS entity_facts_id_seq START 1")

    conn.execute('''
        CREATE TABLE IF NOT EXISTS entity_facts (
            id BIGINT PRIMARY KEY,
            entity_id BIGINT NOT NULL,
            relation VARCHAR NOT NULL,
            value TEXT NOT NULL,
            target_entity_id BIGINT,
            valid_from TIMESTAMPTZ NOT NULL,
            valid_to TIMESTAMPTZ,
            source_note_id BIGINT NOT NULL,
            confidence DOUBLE DEFAULT 0.7,
            metadata JSON
        )
    ''')

    conn.execute('''
        CREATE INDEX IF NOT EXISTS idx_entity_facts_lookup
        ON entity_facts(entity_id, relation)
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
    conn.execute("CREATE INDEX IF NOT EXISTS idx_notes_created ON notes(created DESC)")
    conn.execute("CREATE INDEX IF NOT EXISTS idx_notes_pinned ON notes(pinned DESC, created DESC)")
    conn.execute("CREATE INDEX IF NOT EXISTS idx_notes_pagerank ON notes(pagerank DESC)")
    conn.execute("CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id)")
    conn.execute("CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id)")
    conn.execute("CREATE INDEX IF NOT EXISTS idx_dir_access_time ON directory_access(accessed DESC)")
    
    # Try to set up FTS
    global FTS_ENABLED
    try:
        # DuckDB FTS needs proper installation
        conn.execute("INSTALL fts")
        conn.execute("LOAD fts")
        # Create FTS index for notes table
        try:
            conn.execute("PRAGMA create_fts_index('notes', 'id', 'content', 'summary')")
            FTS_ENABLED = True
            logging.info("DuckDB FTS extension loaded and index created")
        except:
            # Alternative: create a simple FTS table if pragma fails
            conn.execute("""CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts 
                           USING fts(content, summary, content=notes, content_rowid=id)""")
            FTS_ENABLED = True
            logging.info("DuckDB FTS table created")
    except Exception as e:
        FTS_ENABLED = False
        logging.warning(f"FTS not available: {e}")

def migrate_from_sqlite():
    """Migrate from SQLite to DuckDB if needed"""
    if DB_FILE.exists() or not SQLITE_DB_FILE.exists():
        return False
    
    logging.info("Migrating from SQLite to DuckDB...")
    
    try:
        import sqlite3
        sqlite_conn = sqlite3.connect(str(SQLITE_DB_FILE))
        sqlite_conn.row_factory = sqlite3.Row
        
        with _get_db_conn() as duck_conn:
            _create_duckdb_schema(duck_conn)
            
            note_count = sqlite_conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
            logging.info(f"Migrating {note_count} notes...")
            
            duck_conn.execute("BEGIN TRANSACTION")
            
            try:
                # Migrate notes with tags
                notes = sqlite_conn.execute("SELECT * FROM notes").fetchall()
                for note in notes:
                    # Get tags for this note
                    try:
                        tags = sqlite_conn.execute('''
                            SELECT t.name FROM tags t 
                            JOIN note_tags nt ON t.id = nt.tag_id 
                            WHERE nt.note_id = ?
                        ''', (note['id'],)).fetchall()
                        tag_list = [t['name'] for t in tags] if tags else []
                    except sqlite3.OperationalError:
                        tag_list = []
                        if 'tags' in note.keys():
                            tags_data = note['tags']
                            if tags_data:
                                try:
                                    tag_list = json.loads(tags_data) if isinstance(tags_data, str) else []
                                except: 
                                    pass
                    
                    duck_conn.execute('''
                        INSERT INTO notes (
                            id, content, summary, tags, pinned, author,
                            created, session_id, linked_items, pagerank, has_vector
                        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ''', (
                        note['id'], note['content'], note['summary'], tag_list,
                        bool(note['pinned']), note['author'], note['created'],
                        note['session_id'], note['linked_items'],
                        note['pagerank'], bool(note['has_vector'])
                    ))
                
                # Migrate edges
                edges = sqlite_conn.execute("SELECT * FROM edges").fetchall()
                for edge in edges:
                    duck_conn.execute('''
                        INSERT INTO edges (from_id, to_id, type, weight, created)
                        VALUES (?, ?, ?, ?, ?)
                    ''', (edge['from_id'], edge['to_id'], edge['type'],
                          edge['weight'], edge['created']))
                
                # Migrate other tables
                _migrate_simple_table(sqlite_conn, duck_conn, 'entities')
                _migrate_simple_table(sqlite_conn, duck_conn, 'entity_notes')
                _migrate_simple_table(sqlite_conn, duck_conn, 'sessions')
                _migrate_simple_table(sqlite_conn, duck_conn, 'vault')
                _migrate_simple_table(sqlite_conn, duck_conn, 'stats')
                
                duck_conn.execute("COMMIT")
                logging.info("Migration committed successfully!")
                
                max_id = duck_conn.execute("SELECT MAX(id) FROM notes").fetchone()[0]
                if max_id:
                    duck_conn.execute(f"ALTER SEQUENCE notes_id_seq RESTART WITH {max_id + 1}")
                    logging.info(f"Sequence reset to start at {max_id + 1}")
                
            except Exception as e:
                duck_conn.execute("ROLLBACK")
                logging.error(f"Migration failed, rolled back: {e}")
                raise
        
        sqlite_conn.close()
        
        backup_path = SQLITE_DB_FILE.with_suffix(
            f'.backup_{datetime.now().strftime("%Y%m%d%H%M")}.db'
        )
        shutil.move(SQLITE_DB_FILE, backup_path)
        logging.info(f"Old database backed up to {backup_path}")
        
        return True
        
    except Exception as e:
        logging.error(f"Migration failed: {e}", exc_info=True)
        if DB_FILE.exists():
            os.remove(DB_FILE)
        sys.exit(1)

def init_db():
    """Initialize DuckDB database"""
    migrate_from_sqlite()

    with _get_db_conn() as conn:
        tables = conn.execute("SHOW TABLES").fetchall()
        if not any(t[0] == 'notes' for t in tables):
            logging.info("Creating new database schema...")
            _create_duckdb_schema(conn)
        else:
            # Check for directory_access table (new in v6.2)
            if not any(t[0] == 'directory_access' for t in tables):
                logging.info("Adding directory tracking table...")
                conn.execute('''
                    CREATE TABLE IF NOT EXISTS directory_access (
                        id BIGINT PRIMARY KEY,
                        path TEXT NOT NULL,
                        accessed TIMESTAMPTZ NOT NULL,
                        note_id BIGINT,
                        operation VARCHAR
                    )
                ''')
                conn.execute("CREATE INDEX IF NOT EXISTS idx_dir_access_time ON directory_access(accessed DESC)")
            
            try:
                max_id = conn.execute("SELECT MAX(id) FROM notes").fetchone()[0]
                if max_id:
                    # DuckDB doesn't support setval, just use ALTER SEQUENCE
                    conn.execute(f"ALTER SEQUENCE notes_id_seq RESTART WITH {max_id + 1}")
                    logging.info(f"Reset sequence to start at {max_id + 1}")
            except Exception as e:
                # DuckDB may not support ALTER SEQUENCE - this is expected and non-critical
                # We use manual ID management in remember() so sequence issues don't break functionality
                pass

        _ensure_temporal_graph_schema(conn)
        _ensure_entity_facts_schema(conn)

        load_known_entities(conn)
        
        note_count = conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
        if 'db_ready' not in _logged_once:
            logging.info(f"Database ready with {note_count} notes")
            _logged_once.add('db_ready')

def load_known_entities(conn: duckdb.DuckDBPyConnection):
    """Load known entities into memory cache"""
    global KNOWN_ENTITIES
    try:
        entities = conn.execute('SELECT name FROM entities').fetchall()
        KNOWN_ENTITIES = {e[0].lower() for e in entities}
    except:
        KNOWN_ENTITIES = set()

def _ensure_embeddings_loaded():
    """Lazy-load embeddings only when needed (on first semantic search)"""
    global _embeddings_initialized, encoder, chroma_client, collection

    # Check both flag AND encoder object existence (fix for "Dark Memory" bug)
    if _embeddings_initialized and encoder is not None and collection is not None:
        return True

    if 'embedding_init' not in _logged_once:
        logging.info("Lazy-loading embedding model (first semantic search)...")
        _logged_once.add('embedding_init')
    encoder_result = _init_embedding_model()
    vector_result = _init_vector_db()

    # Only set initialized flag if embeddings actually loaded successfully
    if encoder is not None and collection is not None:
        _embeddings_initialized = True

    return encoder is not None

def _init_embedding_model():
    """Initialize embedding model - automatically discover local models first"""
    global encoder, EMBEDDING_MODEL

    if not ST_AVAILABLE or not USE_SEMANTIC:
        logging.debug("Semantic search disabled")
        return None
    
    try:
        # First check for ANY local models in the models folder
        # Check both tools/models and parent models directory
        models_dirs = [
            Path(__file__).parent / "models",
            Path(__file__).parent.parent / "models"
        ]

        for models_dir in models_dirs:
            if not (models_dir.exists() and models_dir.is_dir()):
                continue

            # Find all subdirectories that might contain models
            for model_folder in models_dir.iterdir():
                if model_folder.is_dir():
                    # Check if it looks like a valid model folder
                    # Should have config.json and either pytorch_model.bin or model.safetensors
                    config_file = model_folder / "config.json"
                    has_model = (
                        config_file.exists() and
                        ((model_folder / "model.safetensors").exists() or
                         (model_folder / "pytorch_model.bin").exists())
                    )

                    if has_model:
                        try:
                            model_name = model_folder.name
                            if 'model_discovery' not in _logged_once:
                                logging.info(f"Found local model: {model_name} at {model_folder}")
                                _logged_once.add('model_discovery')
                            if 'model_load_attempt' not in _logged_once:
                                logging.info(f"Attempting to load {model_name}...")
                                _logged_once.add('model_load_attempt')
                            encoder = SentenceTransformer(str(model_folder), device='cpu')
                            test = encoder.encode("test", convert_to_numpy=True)
                            projected = _project_embedding(np.asarray(test))
                            EMBEDDING_MODEL = f'local-{model_name}'
                            if 'model_success' not in _logged_once:
                                logging.info(f"✓ Successfully loaded local model: {model_name}")
                                logging.info(
                                    f"✓ Embedding dimensions: original {np.asarray(test).shape[0]} → active {projected.shape[0]}"
                                )
                                _logged_once.add('model_success')
                            return encoder
                        except Exception as e:
                            logging.warning(f"Failed to load local model {model_name}: {e}")
                            continue
        
        # Fall back to downloading models if no local models work
        logging.info("No local models found or loaded, trying online models...")
        models = [
            ('nomic-ai/nomic-embed-text-v1.5', 'nomic-1.5'),
            ('mixedbread-ai/mxbai-embed-large-v1', 'mxbai'),
            ('BAAI/bge-small-en-v1.5', 'bge-small'),
            ('sentence-transformers/all-MiniLM-L6-v2', 'minilm'),
            ('BAAI/bge-base-en-v1.5', 'bge-base'),
        ]
        
        for model_name, short_name in models:
            try:
                logging.info(f"Loading {model_name}...")
                encoder = SentenceTransformer(model_name, device='cpu')
                test = encoder.encode("test", convert_to_numpy=True)
                projected = _project_embedding(np.asarray(test))
                EMBEDDING_MODEL = short_name
                logging.info(
                    f"✓ Using {short_name} (dim: original {np.asarray(test).shape[0]} → active {projected.shape[0]})"
                )
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
        chroma_client = chromadb.PersistentClient(
            path=str(VECTOR_DIR),
            settings=Settings(anonymized_telemetry=False, allow_reset=True)
        )
        # Use model-specific collection to avoid embedding mismatches
        collection_name = f"notebook_v6_{EMBEDDING_MODEL or 'default'}"
        collection = chroma_client.get_or_create_collection(
            name=collection_name,
            metadata={"hnsw:space": "cosine"}
        )
        if 'chromadb_init' not in _logged_once:
            logging.info(f"ChromaDB initialized with {collection.count()} vectors")
            _logged_once.add('chromadb_init')
        return True
    except Exception as e:
        logging.error(f"ChromaDB init failed: {e}")
        return False

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

def _upsert_edge(
    conn: duckdb.DuckDBPyConnection,
    from_id: int,
    to_id: int,
    edge_type: str,
    weight: float,
    created: datetime,
    source_note_id: Optional[int],
    metadata: Optional[Dict[str, Any]] = None,
) -> None:
    """Insert or refresh an edge with temporal validity tracking."""

    metadata_json = json.dumps(metadata) if metadata else None

    conn.execute('''
        INSERT INTO edges (from_id, to_id, type, weight, created, valid_from, valid_to, source_note_id, metadata)
        VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)
        ON CONFLICT(from_id, to_id, type) DO UPDATE SET
            weight = excluded.weight,
            created = excluded.created,
            source_note_id = COALESCE(excluded.source_note_id, edges.source_note_id),
            metadata = COALESCE(excluded.metadata, edges.metadata),
            valid_from = CASE
                WHEN edges.valid_from IS NULL OR excluded.created < edges.valid_from
                    THEN excluded.created
                ELSE edges.valid_from
            END,
            valid_to = NULL
    ''', [
        from_id,
        to_id,
        edge_type,
        float(weight),
        created,
        created,
        source_note_id,
        metadata_json,
    ])


def _create_all_edges(note_id: int, content: str, session_id: Optional[int], conn: duckdb.DuckDBPyConnection):
    """Create all edge types efficiently - backend only, never shown"""
    now = datetime.now()

    def link_edge(
        from_id: int,
        to_id: int,
        edge_type: str,
        weight: float,
        reason: str,
        extra: Optional[Dict[str, Any]] = None,
    ) -> None:
        payload = {"reason": reason}
        if extra:
            payload.update(extra)
        _upsert_edge(
            conn,
            from_id,
            to_id,
            edge_type,
            weight,
            now,
            note_id,
            payload,
        )

    # Temporal edges
    prev_notes = conn.execute(
        'SELECT id FROM notes WHERE id < ? ORDER BY id DESC LIMIT ?',
        [note_id, TEMPORAL_EDGES]
    ).fetchall()
    for prev in prev_notes:
        link_edge(note_id, prev[0], 'temporal', 1.0, 'temporal_neighbor')
        link_edge(prev[0], note_id, 'temporal', 1.0, 'temporal_neighbor')
    
    # Reference edges
    refs = _extract_references(content)
    if refs:
        # Security: placeholders generated from list length (safe)
        placeholders = ','.join(['?'] * len(refs))
        valid_refs = conn.execute(
            f'SELECT id FROM notes WHERE id IN ({placeholders})',
            refs
        ).fetchall()
        for ref_id in valid_refs:
            link_edge(note_id, ref_id[0], 'reference', 2.0, 'note_reference')
            link_edge(ref_id[0], note_id, 'referenced_by', 2.0, 'note_reference')
    
    # Session edges
    if session_id:
        session_notes = conn.execute(
            'SELECT id FROM notes WHERE session_id = ? AND id != ?',
            [session_id, note_id]
        ).fetchall()
        for other in session_notes:
            link_edge(note_id, other[0], 'session', 1.5, 'session_peer', {"session_id": session_id})
            link_edge(other[0], note_id, 'session', 1.5, 'session_peer', {"session_id": session_id})

    # Entity edges
    entities = _extract_entities(content)
    entity_map: Dict[str, int] = {}
    for entity_name, entity_type in entities:
        entity = conn.execute(
            'SELECT id FROM entities WHERE name = ?',
            [entity_name]
        ).fetchone()

        if entity:
            entity_id = entity[0]
            conn.execute(
                'UPDATE entities SET last_seen = ?, mention_count = mention_count + 1 WHERE id = ?',
                [now, entity_id]
            )
        else:
            max_entity_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM entities").fetchone()[0]
            entity_id = max_entity_id + 1

            conn.execute(
                'INSERT INTO entities (id, name, type, first_seen, last_seen) VALUES (?, ?, ?, ?, ?)',
                [entity_id, entity_name, entity_type, now, now]
            )
            if entity_id:
                KNOWN_ENTITIES.add(entity_name.lower())

        if entity_id:
            entity_map[entity_name.lower()] = entity_id

        if entity_id:
            conn.execute(
                'INSERT INTO entity_notes (entity_id, note_id) VALUES (?, ?) ON CONFLICT DO NOTHING',
                [entity_id, note_id]
            )

            other_notes = conn.execute(
                'SELECT note_id FROM entity_notes WHERE entity_id = ? AND note_id != ?',
                [entity_id, note_id]
            ).fetchall()
            for other in other_notes:
                link_edge(note_id, other[0], 'entity', 1.2, 'shared_entity', {"entity_id": entity_id})
                link_edge(other[0], note_id, 'entity', 1.2, 'shared_entity', {"entity_id": entity_id})

    if entity_map:
        _record_entity_facts(conn, note_id, now, content, entity_map)


def _normalize_entity_key(name: str) -> str:
    return re.sub(r"\s+", " ", str(name or "").strip()).lower()


def _resolve_entity_id(entity_map: Dict[str, int], candidate: str) -> Optional[int]:
    normalized = _normalize_entity_key(candidate)
    if not normalized:
        return None

    if normalized in entity_map:
        return entity_map[normalized]

    for key, value in entity_map.items():
        if normalized in key or key in normalized:
            return value
    return None


def _extract_structured_facts(content: str, entity_map: Dict[str, int]) -> List[Dict[str, Any]]:
    facts: List[Dict[str, Any]] = []
    if not content or not entity_map:
        return facts

    for definition in FACT_PATTERNS:
        relation = definition.get("relation")
        patterns = definition.get("patterns", [])
        invalidate = bool(definition.get("invalidate"))
        confidence = float(definition.get("confidence", 0.7))

        for pattern in patterns:
            try:
                regex = re.compile(pattern, re.IGNORECASE)
            except re.error:
                continue

            for match in regex.finditer(content):
                subject = match.groupdict().get("subject") or ""
                obj = match.groupdict().get("object") or ""
                subject_id = _resolve_entity_id(entity_map, subject)
                if not subject_id:
                    continue

                target_entity_id = _resolve_entity_id(entity_map, obj)
                fact = {
                    "entity_id": subject_id,
                    "relation": relation,
                    "value": obj.strip(),
                    "target_entity_id": target_entity_id,
                    "confidence": confidence,
                    "invalidate": invalidate,
                    "metadata": {
                        "pattern": pattern,
                        "excerpt": match.group(0)[:160],
                        "subject": subject.strip(),
                        "object": obj.strip(),
                    }
                }
                facts.append(fact)

    return facts


def _upsert_entity_fact(
    conn: duckdb.DuckDBPyConnection,
    note_id: int,
    timestamp: datetime,
    fact: Dict[str, Any]
) -> None:
    entity_id = fact.get("entity_id")
    relation = fact.get("relation")
    value = fact.get("value")

    if not entity_id or not relation or not value:
        return

    metadata = fact.get("metadata") or {}
    metadata.setdefault("source_note_id", note_id)
    metadata_json = json.dumps(metadata)

    existing = conn.execute(
        '''
        SELECT id, valid_from, confidence, metadata
        FROM entity_facts
        WHERE entity_id = ? AND relation = ? AND value = ? AND valid_to IS NULL
        ''',
        [entity_id, relation, value]
    ).fetchone()

    if existing:
        existing_id, existing_valid_from, existing_confidence, existing_metadata = existing
        new_valid_from = existing_valid_from if isinstance(existing_valid_from, datetime) else datetime.fromisoformat(str(existing_valid_from))
        if timestamp < new_valid_from:
            new_valid_from = timestamp

        merged_confidence = min(1.0, (float(existing_confidence or 0.7) + float(fact.get("confidence", 0.7))) / 2.0 + 0.05)

        conn.execute(
            '''
            UPDATE entity_facts
            SET valid_from = ?, source_note_id = ?, confidence = ?, metadata = COALESCE(?, metadata)
            WHERE id = ?
            ''',
            [
                new_valid_from,
                note_id,
                merged_confidence,
                metadata_json,
                existing_id,
            ]
        )
        return

    if fact.get("invalidate"):
        conn.execute(
            '''
            UPDATE entity_facts
            SET valid_to = ?
            WHERE entity_id = ? AND relation = ? AND valid_to IS NULL AND value != ?
            ''',
            [timestamp, entity_id, relation, value]
        )

    fact_id = conn.execute("SELECT nextval('entity_facts_id_seq')").fetchone()[0]
    conn.execute(
        '''
        INSERT INTO entity_facts (
            id, entity_id, relation, value, target_entity_id,
            valid_from, valid_to, source_note_id, confidence, metadata
        ) VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)
        ''',
        [
            fact_id,
            entity_id,
            relation,
            value,
            fact.get("target_entity_id"),
            timestamp,
            note_id,
            fact.get("confidence", 0.7),
            metadata_json,
        ]
    )


def _record_entity_facts(
    conn: duckdb.DuckDBPyConnection,
    note_id: int,
    timestamp: datetime,
    content: str,
    entity_map: Dict[str, int]
) -> None:
    facts = _extract_structured_facts(content, entity_map)
    for fact in facts:
        _upsert_entity_fact(conn, note_id, timestamp, fact)


def _fact_query_candidates(
    conn: duckdb.DuckDBPyConnection,
    query: str,
    limit: int
) -> List[Dict[str, Any]]:
    tokens = [tok for tok in re.split(r"\W+", query or "") if len(tok) >= 3]
    if not tokens:
        return []

    like_term = f"%{tokens[0]}%"
    rows = conn.execute(
        '''
        SELECT source_note_id, entity_id, relation, value, confidence
        FROM entity_facts
        WHERE valid_to IS NULL AND (value ILIKE ? OR relation ILIKE ?)
        ORDER BY confidence DESC, valid_from DESC
        LIMIT ?
        ''',
        [like_term, like_term, limit]
    ).fetchall()

    results: List[Dict[str, Any]] = []
    for row in rows:
        if not row or row[0] is None:
            continue
        results.append(
            {
                "note_id": int(row[0]),
                "entity_id": row[1],
                "relation": row[2],
                "value": row[3],
                "confidence": float(row[4] or 0.0),
            }
        )
    return results


def graph_reasoning_candidates(
    conn: duckdb.DuckDBPyConnection,
    query: str,
    seed_ids: List[int],
    limit: int = 20,
    max_hops: int = 2
) -> Dict[str, Any]:
    start = time.time()
    debug: Dict[str, Any] = {
        "status": "started",
        "seeds": list(dict.fromkeys(seed_ids)),
        "max_hops": max_hops,
        "limit": limit,
        "candidates": [],
    }

    if not seed_ids and not query:
        debug.update({"status": "skipped", "reason": "no_seeds"})
        debug["elapsed_ms"] = int((time.time() - start) * 1000)
        return debug

    now = datetime.utcnow()
    queue: deque = deque((seed, 0) for seed in dict.fromkeys(seed_ids))
    visited: Dict[int, int] = {seed: 0 for seed in seed_ids}
    scores: Dict[int, float] = {}
    expansions = 0

    while queue:
        current, depth = queue.popleft()
        if depth >= max_hops:
            continue

        neighbors = conn.execute(
            '''
            SELECT to_id, weight, type, valid_from, valid_to
            FROM edges
            WHERE from_id = ? AND (valid_to IS NULL OR valid_to > ?)
            ''',
            [current, now]
        ).fetchall()

        for to_id, weight, edge_type, valid_from, valid_to in neighbors:
            if to_id == current:
                continue
            hop = depth + 1
            base = float(weight or 1.0)
            decay = 1.0 / (hop + 0.5)
            score = base * decay

            if to_id in scores:
                scores[to_id] = max(scores[to_id], score)
            else:
                scores[to_id] = score

            expansions += 1

            if hop < max_hops and visited.get(to_id, 999) > hop:
                visited[to_id] = hop
                queue.append((to_id, hop))

    fact_matches = _fact_query_candidates(conn, query, limit)
    for match in fact_matches:
        base = 0.6 + 0.4 * match.get("confidence", 0.0)
        scores[match["note_id"]] = max(scores.get(match["note_id"], 0.0), base)

    sorted_candidates = sorted(
        (
            {
                "id": note_id,
                "score": round(score, 6),
            }
            for note_id, score in scores.items()
            if note_id not in seed_ids
        ),
        key=lambda item: item["score"],
        reverse=True,
    )[:limit]

    debug.update(
        {
            "status": "ok" if sorted_candidates else "empty",
            "candidates": sorted_candidates,
            "expansions": expansions,
            "fact_matches": fact_matches[:5],
        }
    )
    debug["elapsed_ms"] = int((time.time() - start) * 1000)
    return debug

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
    global PAGERANK_DIRTY, PAGERANK_CACHE_TIME
    
    count = conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
    if count < 50:
        return
    
    current_time = time.time()
    if PAGERANK_DIRTY or (current_time - PAGERANK_CACHE_TIME > PAGERANK_CACHE_SECONDS):
        calculate_pagerank_duckdb(conn)
        PAGERANK_DIRTY = False
        PAGERANK_CACHE_TIME = current_time

def _log_operation(op: str, dur_ms: int = None):
    """Log operation for stats"""
    try:
        with _get_db_conn() as conn:
            # Get next ID for stats table
            max_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM stats").fetchone()[0]
            new_id = max_id + 1
            
            conn.execute(
                'INSERT INTO stats (id, operation, ts, dur_ms, author) VALUES (?, ?, ?, ?, ?)',
                [new_id, op, datetime.now(), dur_ms, CURRENT_AI_ID]
            )
    except:
        pass

def _log_directory_access(path: str, note_id: Optional[int] = None, operation: Optional[str] = None):
    """Log directory access to database"""
    try:
        with _get_db_conn() as conn:
            max_id = conn.execute("SELECT COALESCE(MAX(id), 0) FROM directory_access").fetchone()[0]
            new_id = max_id + 1
            
            conn.execute('''
                INSERT INTO directory_access (id, path, accessed, note_id, operation)
                VALUES (?, ?, ?, ?, ?)
            ''', [new_id, path, datetime.now(), note_id, operation])
            
            # Also track in memory
            track_directory(path)
            
    except Exception as e:
        logging.debug(f"Could not log directory access: {e}")

def _vacuum_database():
    """Perform VACUUM to reclaim space and optimize database"""
    try:
        logging.info("Starting database VACUUM operation...")
        start_time = time.time()
        
        with _get_db_conn() as conn:
            # Get size before
            size_before = os.path.getsize(DB_FILE)
            
            # Perform VACUUM
            conn.execute("VACUUM")
            
            # Get size after
            size_after = os.path.getsize(DB_FILE)
            
            elapsed = time.time() - start_time
            reduction = size_before - size_after
            percent = (reduction / size_before * 100) if size_before > 0 else 0
            
            logging.info(f"VACUUM completed in {elapsed:.2f}s")
            logging.info(f"Size reduced by {reduction / 1024 / 1024:.1f}MB ({percent:.1f}%)")
            
            return {
                "before_mb": size_before / 1024 / 1024,
                "after_mb": size_after / 1024 / 1024,
                "saved_mb": reduction / 1024 / 1024,
                "percent_saved": percent
            }
            
    except Exception as e:
        logging.error(f"VACUUM failed: {e}")
        return {"error": str(e)}

def get_storage_stats() -> Dict:
    """Get storage statistics"""
    try:
        with _get_db_conn() as conn:
            stats = {
                "db_size_mb": os.path.getsize(DB_FILE) / 1024 / 1024,
                "notes": conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0],
                "edges": conn.execute("SELECT COUNT(*) FROM edges").fetchone()[0],
                "entities": conn.execute("SELECT COUNT(*) FROM entities").fetchone()[0],
                "sessions": conn.execute("SELECT COUNT(*) FROM sessions").fetchone()[0],
                "vectors": collection.count() if collection else 0,
                "recent_dirs": len(RECENT_DIRECTORIES)
            }
            return stats
    except Exception as e:
        logging.error(f"Could not get storage stats: {e}")
        return {}


def store_embedding_vector(
    note_id: int,
    embedding: np.ndarray,
    *,
    content: Optional[str] = None,
    summary: Optional[str] = None,
    tags: Optional[str] = None,
    force: bool = False
) -> Dict[str, Any]:
    """Persist an embedding for a note with optional upsert semantics."""

    if collection is None:
        return {"stored": False, "error": "collection_not_initialized"}

    vector_id = f"note_{note_id}"
    embedding = _project_embedding(embedding)
    document = build_embedding_document(content, summary, tags)
    metadata = {
        "tags": tags or "",
        "summary": (summary or "")[:200]
    }

    payload = {
        "ids": [vector_id],
        "embeddings": [embedding.tolist()],
        "documents": [document],
        "metadatas": [metadata],
    }

    try:
        if force and hasattr(collection, "update"):
            collection.update(**payload)
        else:
            collection.add(**payload)
        return {"stored": True, "vector_id": vector_id, "operation": "update" if force else "add"}
    except Exception as primary_error:
        if not force:
            return {"stored": False, "error": str(primary_error)}

        # Force mode: fall back to delete+add in case update isn't supported
        try:
            collection.delete(ids=[vector_id])
            collection.add(**payload)
            return {
                "stored": True,
                "vector_id": vector_id,
                "operation": "replace",
                "warning": str(primary_error)
            }
        except Exception as secondary_error:
            return {
                "stored": False,
                "error": f"{primary_error}; fallback_failed:{secondary_error}"
            }


def semantic_search(query: str, limit: int = 50) -> Dict[str, Any]:
    """Execute a semantic search and return detailed diagnostics."""

    start = time.time()
    diagnostics: Dict[str, Any] = {
        "query": (query or "").strip(),
        "limit": int(limit or 0) if limit else 0,
        "status": "skipped",
        "ids": [],
        "results": []
    }

    if not diagnostics["query"]:
        diagnostics["reason"] = "empty_query"
        diagnostics["elapsed_ms"] = int((time.time() - start) * 1000)
        return diagnostics

    if not _ensure_embeddings_loaded():
        diagnostics["status"] = "unavailable"
        diagnostics["reason"] = "embeddings_not_initialized"
        diagnostics["elapsed_ms"] = int((time.time() - start) * 1000)
        return diagnostics

    if encoder is None or collection is None:
        diagnostics["status"] = "unavailable"
        diagnostics["reason"] = "encoder_missing"
        diagnostics["elapsed_ms"] = int((time.time() - start) * 1000)
        return diagnostics

    try:
        query_embedding = generate_embedding(diagnostics["query"])
        if query_embedding is None:
            diagnostics.update({
                "status": "skipped",
                "reason": "empty_query_vector"
            })
            diagnostics["elapsed_ms"] = int((time.time() - start) * 1000)
            return diagnostics
        raw_limit = max(1, min(int(limit or 10), 100))
        results = collection.query(
            query_embeddings=[query_embedding.tolist()],
            n_results=raw_limit
        )

        ids = []
        enriched: List[Dict[str, Any]] = []
        raw_ids = results.get("ids") or []
        raw_distances = results.get("distances") or []

        if raw_ids:
            for rank, note_id in enumerate(raw_ids[0]):
                try:
                    numeric_id = int(str(note_id).replace("note_", ""))
                except ValueError:
                    continue

                score_entry: Dict[str, Any] = {
                    "id": numeric_id,
                    "rank": rank + 1,
                }

                if raw_distances and raw_distances[0] and len(raw_distances[0]) > rank:
                    try:
                        distance = float(raw_distances[0][rank])
                        score_entry["distance"] = distance
                        score_entry["score"] = round(max(0.0, 1.0 - distance), 6)
                    except Exception:
                        pass

                ids.append(numeric_id)
                enriched.append(score_entry)

        diagnostics.update({
            "status": "ok",
            "ids": ids[:raw_limit],
            "results": enriched[:raw_limit],
            "vector_count": collection.count() if collection else 0,
            "embedding_model": EMBEDDING_MODEL,
            "embedding_dim": ACTIVE_EMBED_DIM,
        })
    except Exception as exc:
        diagnostics.update({
            "status": "error",
            "error": str(exc)
        })

    diagnostics["elapsed_ms"] = int((time.time() - start) * 1000)
    return diagnostics


def get_search_health(sample_limit: int = 5) -> Dict[str, Any]:
    """Return a consolidated view of notebook search readiness."""

    snapshot: Dict[str, Any] = {
        "embedding_model": EMBEDDING_MODEL,
        "active_embedding_dim": ACTIVE_EMBED_DIM,
        "target_embedding_dim": TARGET_EMBED_DIMS.get(EMBEDDING_MODEL or ""),
        "fts_enabled": bool(FTS_ENABLED),
        "vector_index_ready": bool(collection is not None),
    }

    try:
        snapshot["vector_index_size"] = collection.count() if collection else 0
    except Exception as exc:
        snapshot["vector_index_size_error"] = str(exc)

    try:
        with _get_db_conn() as conn:
            total_notes = conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]
            missing_vectors = conn.execute(
                "SELECT COUNT(*) FROM notes WHERE has_vector = FALSE"
            ).fetchone()[0]
            pinned_notes = conn.execute(
                "SELECT COUNT(*) FROM notes WHERE pinned = TRUE"
            ).fetchone()[0]

            sample_rows = conn.execute(
                "SELECT id, created FROM notes WHERE has_vector = FALSE ORDER BY created DESC LIMIT ?",
                [max(0, int(sample_limit or 0))]
            ).fetchall()

        snapshot.update({
            "total_notes": total_notes,
            "missing_vectors": missing_vectors,
            "pinned_notes": pinned_notes,
            "missing_vector_samples": [
                {"id": row[0], "created": row[1].isoformat() if hasattr(row[1], 'isoformat') else str(row[1])}
                for row in sample_rows
            ]
        })
    except Exception as exc:
        snapshot["database_error"] = str(exc)

    return snapshot

# Initialize storage on module import
init_db()
# NOTE: Embeddings are now lazy-loaded on first use via ensure_embeddings_loaded()
# This saves 7+ seconds on startup when semantic search isn't needed
