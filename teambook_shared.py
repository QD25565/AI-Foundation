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
from pathlib import Path
from datetime import datetime, timedelta, timezone
from typing import Optional, Dict, Any, Tuple, List

# Import from shared MCP utilities
import sys
from pathlib import Path
# Add parent directory to path to find mcp_shared in src/
sys.path.insert(0, str(Path(__file__).parent.parent))

from mcp_shared import (
    BASE_DATA_DIR, CURRENT_AI_ID as MCP_AI_ID,
    pipe_escape, format_time_compact, get_tool_data_dir,
    normalize_param
)

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
TEAMBOOK_ROOT_ENV = os.environ.get('TEAMBOOK_ROOT', None)
if TEAMBOOK_ROOT_ENV:
    TEAMBOOK_ROOT = Path(TEAMBOOK_ROOT_ENV)
else:
    # DEFAULT: Local user directory
    # For multi-instance coordination, set TEAMBOOK_ROOT environment variable to a shared location
    TEAMBOOK_ROOT = Path.home() / ".teambook"
TEAMBOOK_PRIVATE_ROOT = TEAMBOOK_ROOT / "_private"

# Create root directories
TEAMBOOK_ROOT.mkdir(parents=True, exist_ok=True)
TEAMBOOK_PRIVATE_ROOT.mkdir(parents=True, exist_ok=True)

# ============= PATH MANAGEMENT =============
def get_data_dir():
    """Get current data directory based on teambook context"""
    if CURRENT_TEAMBOOK:
        team_dir = TEAMBOOK_ROOT / CURRENT_TEAMBOOK
        team_dir.mkdir(parents=True, exist_ok=True)
        return team_dir
    return TEAMBOOK_PRIVATE_ROOT

def get_db_file():
    """Get current database file path"""
    return get_data_dir() / "teambook.duckdb"

def get_outputs_dir():
    """Get outputs directory for evolution results"""
    outputs = get_data_dir() / "outputs"
    outputs.mkdir(parents=True, exist_ok=True)
    return outputs

def set_current_teambook(name):
    """Set and persist the current active teambook"""
    global CURRENT_TEAMBOOK
    CURRENT_TEAMBOOK = name
    # Save to file so it persists across commands
    context_file = TEAMBOOK_ROOT / ".current_teambook"
    context_file.write_text(name)

def load_current_teambook():
    """Load persisted teambook context from file"""
    global CURRENT_TEAMBOOK
    context_file = TEAMBOOK_ROOT / ".current_teambook"
    if context_file.exists():
        persisted = context_file.read_text().strip()
        if persisted and not CURRENT_TEAMBOOK:  # Only load if not already set by env var
            CURRENT_TEAMBOOK = persisted
    else:
        # Default to town-hall-qd if no teambook set
        if not CURRENT_TEAMBOOK:
            CURRENT_TEAMBOOK = "town-hall-qd"
            # Create the file so all instances use same default
            try:
                context_file.write_text("town-hall-qd")
            except:
                pass

def get_vault_key_file():
    """Get vault key file path"""
    return get_data_dir() / ".vault_key"

def get_last_op_file():
    """Get last operation file path"""
    return get_data_dir() / ".last_operation"

def get_vector_dir():
    """Get vector database directory"""
    return get_data_dir() / "vectors"

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
