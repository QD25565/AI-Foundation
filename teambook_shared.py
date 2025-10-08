#!/usr/bin/env python3
"""
TEAMBOOK SHARED v1.0.0 - SHARED UTILITIES AND CONSTANTS
========================================================
Core utilities, constants, and shared state for the teambook collaborative tool.
This is the foundation layer with no dependencies on other teambook modules.

Built by AIs, for AIs.
========================================================
"""

import os
import sys
import re
import json
import random
import logging
import hashlib
from pathlib import Path
from datetime import datetime, timedelta, timezone
from typing import Optional, Dict, Any, Tuple, List, Mapping, Callable

# Import from shared MCP utilities
PARENT_DIR = Path(__file__).parent.parent
if str(PARENT_DIR) not in sys.path:
    sys.path.insert(0, str(PARENT_DIR))

from mcp_shared import (
    BASE_DATA_DIR, CURRENT_AI_ID as MCP_AI_ID,
    pipe_escape, format_time_compact, get_tool_data_dir,
    normalize_param
)

# ============= ENTERPRISE CONFIGURATION =============
_ENTERPRISE_CONFIG_PATH: Optional[Path] = None
_ENTERPRISE_CONFIG: Dict[str, Any] = {}
_AI_SIGNATURE_SECRET: Optional[bytes] = None


def attempt_with_grace(feature_name: str, action: Callable[[], Any], fallback: Any = None) -> Any:
    """Execute an action and fall back gracefully on failure."""

    try:
        return action()
    except Exception as exc:
        logging.warning(f"{feature_name} unavailable: {exc}. Falling back to safe defaults.")
        return fallback


@contextmanager
def graceful_degradation(feature_name: str):
    """Context manager for optional features that should never crash callers."""

    try:
        yield
    except Exception as exc:
        logging.warning(f"{feature_name} degraded: {exc}")


def _discover_config_path() -> Optional[Path]:
    """Locate the enterprise configuration file if present."""

    env_path = os.environ.get('TEAMBOOK_CONFIG_PATH')
    candidates: List[Path] = []

    if env_path:
        candidates.append(Path(env_path).expanduser())

    repo_candidate = PARENT_DIR / "teambook.enterprise.json"
    candidates.append(repo_candidate)
    candidates.append(Path.cwd() / "teambook.enterprise.json")
    candidates.append(BASE_DATA_DIR / "teambook.enterprise.json")
    candidates.append(Path.home() / ".config" / "teambook" / "teambook.enterprise.json")

    for candidate in candidates:
        if candidate and candidate.is_file():
            return candidate

    return None


def _load_config_file(path: Path) -> Dict[str, Any]:
    """Load enterprise configuration data from disk."""

    if path.suffix.lower() != '.json':
        raise ValueError(f"Unsupported config format: {path.suffix}. Expected JSON.")

    with path.open('r', encoding='utf-8') as handle:
        return json.load(handle)


def load_enterprise_config(refresh: bool = False) -> Dict[str, Any]:
    """Load and cache enterprise configuration with graceful fallback."""

    global _ENTERPRISE_CONFIG, _ENTERPRISE_CONFIG_PATH

    if not refresh and _ENTERPRISE_CONFIG:
        return _ENTERPRISE_CONFIG

    _ENTERPRISE_CONFIG_PATH = attempt_with_grace(
        "config_discovery",
        _discover_config_path
    )

    if not _ENTERPRISE_CONFIG_PATH:
        _ENTERPRISE_CONFIG = {}
        return _ENTERPRISE_CONFIG

    _ENTERPRISE_CONFIG = attempt_with_grace(
        f"config_load:{_ENTERPRISE_CONFIG_PATH}",
        lambda: _load_config_file(_ENTERPRISE_CONFIG_PATH),
        {}
    ) or {}

    return _ENTERPRISE_CONFIG


def get_enterprise_setting(path: str, default: Any = None) -> Any:
    """Retrieve nested configuration values using dot-separated paths."""

    config = load_enterprise_config()
    current: Any = config

    for part in path.split('.'):
        if not isinstance(current, dict) or part not in current:
            return default
        current = current[part]

    return current


def ensure_directory(path: Path, fallback: Optional[Path] = None, label: str = "data") -> Path:
    """Create a directory with graceful fallback to temp storage."""

    try:
        path.mkdir(parents=True, exist_ok=True)
        return path
    except Exception as exc:
        logging.warning(f"Unable to prepare {label} directory at {path}: {exc}")
        if fallback:
            try:
                fallback.mkdir(parents=True, exist_ok=True)
                return fallback
            except Exception as fallback_exc:
                logging.warning(f"Fallback {label} directory failed at {fallback}: {fallback_exc}")

        temp_dir = Path(tempfile.gettempdir()) / "teambook"
        temp_dir.mkdir(parents=True, exist_ok=True)
        return temp_dir


def safe_write_text(path: Path, data: str) -> None:
    """Write text to disk without raising fatal errors."""

    def _write() -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(data, encoding='utf-8')

    attempt_with_grace(f"write:{path}", _write, None)


def safe_read_text(path: Path) -> Optional[str]:
    """Read text from disk with graceful fallback."""

    if not path.exists():
        return None

    return attempt_with_grace(
        f"read:{path}",
        lambda: path.read_text(encoding='utf-8'),
        None
    )


def _slugify_token(token: str) -> str:
    """Normalize tokens for use in teambook identifiers."""

    cleaned = re.sub(r'[^a-zA-Z0-9_-]', '-', token.strip())
    cleaned = re.sub(r'-{2,}', '-', cleaned)
    return cleaned.lower().strip('-') or 'node'


def get_hostname_token() -> str:
    """Derive a deterministic hostname token."""

    import socket
    import platform

    hostname = _slugify_token(socket.gethostname())

    if hostname in {'localhost', 'desktop', 'laptop', 'pc'}:
        system = _slugify_token(platform.system())
        hostname = f"{hostname}-{system}" if system else f"{hostname}-sys"

    return hostname or 'node'


def get_default_teambook_name(scope: Optional[str] = None) -> str:
    """Compute the default teambook name honoring enterprise configuration."""

    configured = get_enterprise_setting('defaults.teambook')
    if configured:
        return str(configured)

    scope_override = scope or os.environ.get('TOWN_HALL_SCOPE') or get_enterprise_setting('defaults.town_hall.scope', 'computer')
    scope_normalized = str(scope_override or 'computer').lower()

    if scope_normalized == 'universal':
        return get_enterprise_setting('defaults.town_hall.name', 'town-hall')

    if scope_normalized == 'ai':
        return f"town-hall-{_slugify_token(CURRENT_AI_ID or 'ai')}"

    base_name = get_enterprise_setting('defaults.town_hall.name')
    if base_name:
        return str(base_name)

    return f"town-hall-{get_hostname_token()}"

# ============= VERSION AND CONFIGURATION =============
VERSION = "1.0.0"
OUTPUT_FORMAT = os.environ.get('TEAMBOOK_FORMAT', 'pipe')
USE_SEMANTIC = os.environ.get('TEAMBOOK_SEMANTIC', 'true').lower() == 'true'
TEAMBOOK_NAME_ENV = os.environ.get('TEAMBOOK_NAME', None)  # Shared teambook name from environment

# ============= EXECUTION CONTEXT DETECTION =============
def get_execution_context() -> str:
    """
    Detect if running in CLI or MCP mode.

    Returns:
        'mcp' if called from MCP server
        'cli' if called from command line
    """
    # Method 1: Check for MCP-specific environment variable
    if os.environ.get('MCP_SERVER_MODE') == 'true':
        return 'mcp'

    # Method 2: Check if __main__ module is ai_foundation_server
    main_module = sys.modules.get('__main__')
    if main_module and hasattr(main_module, '__file__'):
        main_file = main_module.__file__
        if main_file and 'ai_foundation_server' in main_file:
            return 'mcp'

    # Method 3: Check for asyncio event loop (MCP uses async)
    try:
        import asyncio
        if asyncio.get_event_loop().is_running():
            return 'mcp'
    except:
        pass

    # Default to CLI
    return 'cli'

# Cache context detection (won't change during execution)
EXECUTION_CONTEXT = get_execution_context()
IS_MCP = EXECUTION_CONTEXT == 'mcp'
IS_CLI = EXECUTION_CONTEXT == 'cli'

# ============= LIMITS AND CONSTANTS =============
MAX_CONTENT_LENGTH = 5000
MAX_SUMMARY_LENGTH = 200
MAX_RESULTS = 100
BATCH_MAX = 50
DEFAULT_RECENT = 30

# Edge and PageRank settings
TEMPORAL_EDGES = 3
SESSION_GAP_MINUTES = 30
PAGERANK_ITERATIONS = 20
PAGERANK_DAMPING = 0.85
PAGERANK_CACHE_SECONDS = 300
ATTEMPT_CLEANUP_HOURS = 24

# ============= GLOBAL STATE =============
CURRENT_TEAMBOOK = TEAMBOOK_NAME_ENV  # Set from environment variable if provided
CURRENT_AI_ID = MCP_AI_ID  # Use shared AI identity
LAST_OPERATION = None
_FEDERATION_SECRET = None

# Knowledge bases
KNOWN_ENTITIES = set()
KNOWN_TOOLS = {
    'teambook', 'firebase', 'gemini', 'claude', 'jetbrains', 'github',
    'slack', 'discord', 'vscode', 'git', 'docker', 'python', 'node',
    'react', 'vue', 'angular', 'tensorflow', 'pytorch', 'aws', 'gcp',
    'azure', 'kubernetes', 'redis', 'postgres', 'mongodb', 'sqlite',
    'task_manager', 'notebook', 'world', 'chromadb', 'duckdb'
}

# Pattern caching
ENTITY_PATTERN = None
ENTITY_PATTERN_SIZE = 0
PAGERANK_DIRTY = True
PAGERANK_CACHE_TIME = 0

# ============= CROSS-PLATFORM DIRECTORY STRUCTURE =============
# CENTRAL SHARED TEAMBOOK LOCATION
# All instances connect to the SAME network location
_TEAMBOOK_ROOT_SETTING = os.environ.get(
    'TEAMBOOK_ROOT',
    get_enterprise_setting('paths.root', str(Path.home() / '.teambook'))
)
TEAMBOOK_ROOT = ensure_directory(Path(str(_TEAMBOOK_ROOT_SETTING)).expanduser(), label='teambook_root')

_TEAMBOOK_PRIVATE_SETTING = get_enterprise_setting('paths.private_root')
_TEAMBOOK_PRIVATE_CANDIDATE = (
    Path(str(_TEAMBOOK_PRIVATE_SETTING)).expanduser()
    if _TEAMBOOK_PRIVATE_SETTING
    else TEAMBOOK_ROOT / '_private'
)
TEAMBOOK_PRIVATE_ROOT = ensure_directory(
    _TEAMBOOK_PRIVATE_CANDIDATE,
    fallback=TEAMBOOK_ROOT / '_private',
    label='teambook_private_root'
)

# ============= PATH MANAGEMENT =============
def get_data_dir():
    """Get current data directory based on teambook context"""
    if CURRENT_TEAMBOOK:
        team_dir = ensure_directory(
            TEAMBOOK_ROOT / CURRENT_TEAMBOOK,
            fallback=TEAMBOOK_PRIVATE_ROOT / CURRENT_TEAMBOOK,
            label=f'teambook:{CURRENT_TEAMBOOK}'
        )
        return team_dir
    return TEAMBOOK_PRIVATE_ROOT

def get_db_file():
    """Get current database file path"""
    return get_data_dir() / "teambook.duckdb"

def get_outputs_dir():
    """Get outputs directory for evolution results"""
    outputs = ensure_directory(get_data_dir() / "outputs", label='outputs')
    return outputs

def set_current_teambook(name):
    """Set and persist the current active teambook"""
    global CURRENT_TEAMBOOK
    normalized = str(name).strip() if name else None
    CURRENT_TEAMBOOK = normalized
    # Save to file so it persists across commands
    context_file = TEAMBOOK_ROOT / ".current_teambook"
    if normalized:
        safe_write_text(context_file, normalized)
    else:
        def _remove():
            if context_file.exists():
                context_file.unlink()

        attempt_with_grace('clear_current_teambook', _remove, None)

def load_current_teambook():
    """Load persisted teambook context from file"""
    global CURRENT_TEAMBOOK
    context_file = TEAMBOOK_ROOT / ".current_teambook"
    persisted = safe_read_text(context_file)
    if persisted and not CURRENT_TEAMBOOK:
        CURRENT_TEAMBOOK = persisted.strip()

    if not CURRENT_TEAMBOOK:
        default_teambook = get_default_teambook_name()
        CURRENT_TEAMBOOK = default_teambook
        safe_write_text(context_file, default_teambook)


# ============= SECURITY & SIGNATURES =============
def _get_ai_signature_path(ai_id: Optional[str] = None) -> Path:
    """Return the file path storing the AI signature secret."""

    token = _slugify_token(ai_id or CURRENT_AI_ID or 'ai')
    return TEAMBOOK_PRIVATE_ROOT / f"{token}.signing"


def _apply_secure_permissions(path: Path) -> None:
    """Apply restrictive permissions to sensitive files when possible."""

    if os.name == 'posix':
        attempt_with_grace('chmod_signature_secret', lambda: os.chmod(path, 0o600), None)


def get_ai_signature_secret(force_refresh: bool = False) -> Optional[bytes]:
    """Load or generate the per-AI signature secret."""

    global _AI_SIGNATURE_SECRET

    if _AI_SIGNATURE_SECRET is not None and not force_refresh:
        return _AI_SIGNATURE_SECRET

    secret_path = _get_ai_signature_path()

    def _read_secret() -> Optional[bytes]:
        if not secret_path.exists():
            return None
        encoded = secret_path.read_text(encoding='utf-8').strip()
        return base64.b64decode(encoded.encode('utf-8')) if encoded else None

    secret = attempt_with_grace('signature_secret_read', _read_secret, None)

    if not secret:
        secret = secrets.token_bytes(32)

        def _write_secret() -> None:
            secret_path.parent.mkdir(parents=True, exist_ok=True)
            secret_path.write_text(base64.b64encode(secret).decode('utf-8'), encoding='utf-8')
            _apply_secure_permissions(secret_path)

        attempt_with_grace('signature_secret_write', _write_secret, None)

    _AI_SIGNATURE_SECRET = secret
    return _AI_SIGNATURE_SECRET


def _serialize_payload(payload_fields: Mapping[str, Any]) -> str:
    """Serialize payload deterministically for hashing/signing."""

    def _default(obj: Any) -> Any:
        if isinstance(obj, (datetime, timedelta)):
            base = obj
            if isinstance(obj, timedelta):
                base = datetime(1970, 1, 1, tzinfo=timezone.utc) + obj
            return base.isoformat()
        if isinstance(obj, Path):
            return str(obj)
        return obj

    return json.dumps(payload_fields, default=_default, sort_keys=True, separators=(',', ':'))


def build_security_envelope(payload_fields: Mapping[str, Any], purpose: str) -> Dict[str, Any]:
    """Create a signed envelope describing a payload."""

    issued_at = datetime.now(timezone.utc)
    serialized = _serialize_payload(payload_fields)
    payload_hash = hashlib.sha3_256(serialized.encode('utf-8')).hexdigest()
    secret = get_ai_signature_secret()

    envelope = {
        'ai_id': CURRENT_AI_ID,
        'purpose': purpose,
        'issued_at': issued_at.isoformat(),
        'payload_hash': payload_hash,
        'status': 'unsigned'
    }

    if secret:
        signature_payload = json.dumps({
            'ai_id': envelope['ai_id'],
            'purpose': envelope['purpose'],
            'issued_at': envelope['issued_at'],
            'payload_hash': envelope['payload_hash']
        }, sort_keys=True, separators=(',', ':'))
        envelope['signature'] = hmac.new(secret, signature_payload.encode('utf-8'), hashlib.sha3_256).hexdigest()
        envelope['status'] = 'signed'

    return envelope


def verify_security_envelope(payload_fields: Mapping[str, Any], envelope: Dict[str, Any]) -> bool:
    """Verify that the provided envelope matches the payload."""

    if not envelope or envelope.get('status') != 'signed':
        return False

    expected_hash = hashlib.sha3_256(_serialize_payload(payload_fields).encode('utf-8')).hexdigest()
    if envelope.get('payload_hash') != expected_hash:
        return False

    secret = get_ai_signature_secret()
    if not secret:
        return False

    signature_payload = json.dumps({
        'ai_id': envelope.get('ai_id'),
        'purpose': envelope.get('purpose'),
        'issued_at': envelope.get('issued_at'),
        'payload_hash': envelope.get('payload_hash')
    }, sort_keys=True, separators=(',', ':'))

    expected_signature = hmac.new(secret, signature_payload.encode('utf-8'), hashlib.sha3_256).hexdigest()
    provided_signature = envelope.get('signature')
    return isinstance(provided_signature, str) and hmac.compare_digest(expected_signature, provided_signature)


def ensure_metadata_dict(metadata: Any) -> Dict[str, Any]:
    """Normalize metadata payloads to a dictionary structure."""

    if metadata is None:
        return {}

    if isinstance(metadata, dict):
        return dict(metadata)

    if isinstance(metadata, list):
        return {'items': metadata}

    if isinstance(metadata, str):
        stripped = metadata.strip()
        if not stripped:
            return {}
        try:
            parsed = json.loads(stripped)
            if isinstance(parsed, dict):
                return parsed
            if isinstance(parsed, list):
                return {'items': parsed}
        except json.JSONDecodeError:
            pass
        return {'value': stripped}

    return {'value': metadata}


def attach_security_envelope(metadata: Any, payload_fields: Mapping[str, Any], purpose: str) -> Dict[str, Any]:
    """Merge security envelope information into metadata."""

    metadata_dict = ensure_metadata_dict(metadata)
    envelope = metadata_dict.get('security')

    if not isinstance(envelope, dict) or 'payload_hash' not in envelope:
        metadata_dict['security'] = build_security_envelope(payload_fields, purpose)
    else:
        metadata_dict['security'] = build_security_envelope(payload_fields, purpose)

    identity_hint = metadata_dict.get('identity_hint')
    if identity_hint is None:
        human_identity = get_registered_human_identity()
        if human_identity:
            metadata_dict['identity_hint'] = human_identity

    return metadata_dict


def get_registered_human_identity(ai_id: Optional[str] = None) -> Optional[Dict[str, Any]]:
    """Return configured human identity hints for hybrid collaboration."""

    candidate_keys = []
    ai_token = _slugify_token(ai_id or CURRENT_AI_ID or 'ai')
    if ai_token:
        candidate_keys.append(f'humans.{ai_token}')
    candidate_keys.append('humans.default')

    identity_data: Optional[Dict[str, Any]] = None
    for key in candidate_keys:
        value = get_enterprise_setting(key)
        if isinstance(value, dict):
            identity_data = value
            break

    if not identity_data:
        env_phone = os.environ.get('HUMAN_PHONE')
        env_email = os.environ.get('HUMAN_EMAIL')
        env_handle = os.environ.get('HUMAN_CONTACT')
        identity_data = {
            'phone': env_phone,
            'email': env_email,
            'handle': env_handle
        }

    if not identity_data:
        return None

    cleaned = {k: v for k, v in identity_data.items() if isinstance(v, str) and v.strip()}
    return cleaned or None

def get_vault_key_file():
    """Get vault key file path"""
    return get_data_dir() / ".vault_key"

def get_last_op_file():
    """Get last operation file path"""
    return get_data_dir() / ".last_operation"

def get_vector_dir():
    """Get vector database directory"""
    return get_data_dir() / "vectors"


def get_federation_secret() -> str:
    """Derive a deterministic secret for federation signatures."""
    global _FEDERATION_SECRET

    if _FEDERATION_SECRET is not None:
        return _FEDERATION_SECRET

    env_secret = os.environ.get('TEAMBOOK_FEDERATION_SECRET')
    if env_secret:
        _FEDERATION_SECRET = env_secret.strip()
        return _FEDERATION_SECRET

    try:
        vault_file = get_vault_key_file()
        if vault_file.exists():
            data = vault_file.read_bytes()
            _FEDERATION_SECRET = hashlib.sha256(data).hexdigest()
        else:
            seed = f"{TEAMBOOK_ROOT}|{CURRENT_TEAMBOOK or 'private'}"
            _FEDERATION_SECRET = hashlib.sha256(seed.encode('utf-8')).hexdigest()
    except Exception:
        seed = f"fallback|{CURRENT_TEAMBOOK or 'private'}"
        _FEDERATION_SECRET = hashlib.sha256(seed.encode('utf-8')).hexdigest()

    return _FEDERATION_SECRET

# ============= TEXT UTILITIES =============
def clean_text(text: str) -> str:
    """Clean text by removing extra whitespace"""
    return re.sub(r'\s+', ' ', text).strip() if text else ""

def simple_summary(content: str, max_len: int = 150) -> str:
    """Create simple summary by truncating cleanly"""
    if not content:
        return ""
    clean = clean_text(content)
    if len(clean) <= max_len:
        return clean
    
    # Try to break at sentence boundaries
    for sep in ['. ', '! ', '? ', '; ']:
        idx = clean.rfind(sep, 0, max_len)
        if idx > max_len * 0.5:
            return clean[:idx + 1]
    
    # Fall back to word boundary
    idx = clean.rfind(' ', 0, max_len - 3)
    if idx == -1 or idx < max_len * 0.7:
        idx = max_len - 3
    return clean[:idx] + "..."

# ============= TIME UTILITIES =============
def parse_time_query(when: str) -> Tuple[Optional[datetime], Optional[datetime]]:
    """Parse natural language time queries"""
    if not when:
        return None, None
    
    when_lower = when.lower().strip()
    now = datetime.now(timezone.utc)
    today_start = now.replace(hour=0, minute=0, second=0, microsecond=0)
    
    # Common time queries
    if when_lower == "today":
        return today_start, now
    elif when_lower == "yesterday":
        yesterday_start = today_start - timedelta(days=1)
        return yesterday_start, today_start - timedelta(seconds=1)
    elif when_lower in ["this week", "week"]:
        week_start = today_start - timedelta(days=now.weekday())
        return week_start, now
    elif when_lower == "last week":
        week_start = today_start - timedelta(days=now.weekday())
        last_week_start = week_start - timedelta(days=7)
        return last_week_start, week_start
    
    return None, None

# ============= CONTENT ANALYSIS =============
def extract_references(content: str) -> List[int]:
    """Extract note references from content"""
    refs = set()
    patterns = [
        r'note\s+(\d+)',
        r'\bn(\d+)\b',
        r'#(\d+)\b',
        r'\[(\d+)\]'
    ]
    
    for pattern in patterns:
        matches = re.findall(pattern, content, re.IGNORECASE)
        refs.update(int(m) for m in matches if m.isdigit())
    
    return list(refs)

def extract_entities(content: str) -> List[Tuple[str, str]]:
    """Extract entities from content"""
    global ENTITY_PATTERN, ENTITY_PATTERN_SIZE
    entities = []
    content_lower = content.lower()
    
    # Extract mentions
    mentions = re.findall(r'@([\w-]+)', content, re.IGNORECASE)
    entities.extend((m.lower(), 'mention') for m in mentions)
    
    # Extract known entities
    all_known = KNOWN_TOOLS.union(KNOWN_ENTITIES)
    if all_known:
        if ENTITY_PATTERN is None or len(all_known) != ENTITY_PATTERN_SIZE:
            pattern_str = r'\b(' + '|'.join(re.escape(e) for e in all_known) + r')\b'
            ENTITY_PATTERN = re.compile(pattern_str, re.IGNORECASE)
            ENTITY_PATTERN_SIZE = len(all_known)
        
        if ENTITY_PATTERN:
            for entity_name in set(ENTITY_PATTERN.findall(content_lower)):
                entity_type = 'tool' if entity_name in KNOWN_TOOLS else 'known'
                entities.append((entity_name, entity_type))
    
    return entities

# ============= OPERATION TRACKING =============
def save_last_operation(op_type: str, result: Any):
    """Save last operation for chaining"""
    global LAST_OPERATION
    LAST_OPERATION = {
        'type': op_type,
        'result': result,
        'time': datetime.now(timezone.utc)
    }
    
    try:
        with open(get_last_op_file(), 'w') as f:
            json.dump({
                'type': op_type,
                'time': LAST_OPERATION['time'].isoformat()
            }, f)
    except:
        pass

def get_last_operation() -> Optional[Dict]:
    """Get last operation for context"""
    global LAST_OPERATION
    if LAST_OPERATION:
        return LAST_OPERATION
    
    try:
        last_op_file = get_last_op_file()
        if last_op_file.exists():
            with open(last_op_file, 'r') as f:
                data = json.load(f)
                return {
                    'type': data['type'],
                    'time': datetime.fromisoformat(data['time'])
                }
    except:
        pass
    
    return None

def get_note_id(id_param: Any) -> Optional[int]:
    """
    Resolve note ID to integer

    SECURITY: Strict input validation to prevent type confusion attacks

    Accepts:
    - "last": Returns most recent note ID
    - Integer: Returns as-is
    - String with digits: Extracts integer (e.g., "note:123" -> 123)
    - String "evo:123": Extracts 123

    Returns None for invalid inputs instead of raising exceptions.
    """
    # Handle "last" keyword
    if id_param == "last":
        last_op = get_last_operation()
        if last_op and last_op['type'] in ['remember', 'write']:
            return last_op['result'].get('id')
        return None

    # Already an integer
    if isinstance(id_param, int):
        # SECURITY: Validate positive integer
        return id_param if id_param > 0 else None

    # String input - extract digits only
    if isinstance(id_param, str):
        # SECURITY: Remove all non-digit characters to prevent injection
        clean_id = re.sub(r'[^\d]', '', id_param)
        if not clean_id:
            return None
        try:
            note_id = int(clean_id)
            # SECURITY: Validate positive integer
            return note_id if note_id > 0 else None
        except ValueError:
            return None

    # Attempt conversion for other types
    try:
        note_id = int(id_param)
        return note_id if note_id > 0 else None
    except (ValueError, TypeError):
        return None

# ============= LINEAR MEMORY BRIDGE =============
# Write-through cache: when AI writes to teambook, cache locally for start_session()

CACHE_AVAILABLE = True  # Enable Linear Memory Bridge

def get_my_notes_cache_file():
    """Get path to this AI's cached teambook notes"""
    return TEAMBOOK_PRIVATE_ROOT / f".my_notes_{CURRENT_AI_ID}.json"

def _save_note_to_cache(note_id: int, content: str, summary: str, teambook: str):
    """Save a note to local cache (write-through)
    
    Args:
        note_id: Note ID
        content: Note content  
        summary: Note summary
        teambook: Teambook name (or None for private)
    """
    try:
        cache_file = get_my_notes_cache_file()
        
        # Load existing cache
        cached_notes = []
        if cache_file.exists():
            try:
                with open(cache_file, 'r', encoding='utf-8') as f:
                    cached_notes = json.load(f)
            except:
                pass  # Start fresh if corrupted
        
        # Add new note to front
        cached_notes.insert(0, {
            'id': note_id,
            'summary': summary,
            'content_preview': content[:200],  # First 200 chars
            'teambook': teambook,
            'cached_at': datetime.now(timezone.utc).isoformat()
        })
        
        # Keep only last 10 notes
        cached_notes = cached_notes[:10]
        
        # Save cache
        with open(cache_file, 'w', encoding='utf-8') as f:
            json.dump(cached_notes, f, indent=2)
            
        logging.debug(f"Cached note {note_id} for Linear Memory Bridge")
        
    except Exception as e:
        logging.debug(f"Failed to cache note: {e}")

def load_my_notes_cache() -> Optional[List[Dict]]:
    """Load this AI's cached teambook notes
    
    Returns:
        List of cached note dicts or None if cache doesn't exist
    """
    try:
        cache_file = get_my_notes_cache_file()
        if not cache_file.exists():
            return None
            
        with open(cache_file, 'r', encoding='utf-8') as f:
            return json.load(f)
            
    except Exception as e:
        logging.debug(f"Failed to load note cache: {e}")
        return None

# ============= MODULE INITIALIZATION =============
# Load persisted teambook context on module import
load_current_teambook()
