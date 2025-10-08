#!/usr/bin/env python3
"""
LINEAR MEMORY BRIDGE - Write-Through Cache
===========================================
Cache last 10 teambook notes written by THIS AI instance.
Enables start_session() to show recent contributions without API calls.

Design: Simple write-through cache stored locally.
"""

import json
import logging
from pathlib import Path
from typing import List, Dict, Optional
from datetime import datetime, timezone

# Import shared utilities
try:
    from teambook_shared import CURRENT_AI_ID, CURRENT_TEAMBOOK
except ImportError:
    CURRENT_AI_ID = None
    CURRENT_TEAMBOOK = None

logger = logging.getLogger(__name__)

# Cache configuration
CACHE_SIZE = 10  # Last 10 notes
CACHE_FILENAME = "teambook_my_notes_cache.json"


def get_cache_file() -> Path:
    """Get path to cache file in notebook data directory"""
    # Use notebook data directory for consistency
    try:
        from pathlib import Path
        import os

        # Try to find notebook data directory
        notebook_data = Path.cwd() / "data" / "notebook_data"
        if not notebook_data.exists():
            # Fallback to teambook data directory
            notebook_data = Path.cwd() / "data" / "teambook_data"

        notebook_data.mkdir(parents=True, exist_ok=True)
        return notebook_data / CACHE_FILENAME

    except Exception as e:
        logger.warning(f"Could not determine cache location: {e}")
        # Fallback to current directory
        return Path(CACHE_FILENAME)


def save_note_to_cache(note_id: int, content: str, summary: str,
                       teambook_name: Optional[str] = None) -> None:
    """
    Add a note to the write-through cache.
    Called after successful teambook write.

    Args:
        note_id: The note ID
        content: Note content (truncated to 200 chars for cache)
        summary: Note summary
        teambook_name: Which teambook this was written to
    """
    try:
        cache_file = get_cache_file()

        # Load existing cache
        cache_data = []
        if cache_file.exists():
            try:
                with open(cache_file, 'r', encoding='utf-8') as f:
                    cache_data = json.load(f)
            except Exception as e:
                logger.warning(f"Could not load cache, starting fresh: {e}")
                cache_data = []

        # Create new cache entry
        new_entry = {
            "note_id": note_id,
            "content": content[:200],  # Truncate for cache efficiency
            "summary": summary,
            "teambook": teambook_name or CURRENT_TEAMBOOK or "_private",
            "cached_at": datetime.now(timezone.utc).isoformat(),
            "author": CURRENT_AI_ID
        }

        # Add to beginning of list (most recent first)
        cache_data.insert(0, new_entry)

        # Keep only last N entries
        cache_data = cache_data[:CACHE_SIZE]

        # Save back to file
        with open(cache_file, 'w', encoding='utf-8') as f:
            json.dump(cache_data, f, indent=2)

        logger.debug(f"Cached note {note_id} to {cache_file}")

    except Exception as e:
        logger.warning(f"Failed to save note to cache: {e}")
        # Non-critical failure - cache is just a performance optimization


def load_my_teambook_notes() -> List[Dict]:
    """
    Load cached teambook notes written by this AI.
    Returns list of cached note dicts, most recent first.

    Returns:
        List of note dicts with keys: note_id, content, summary, teambook, cached_at
    """
    try:
        cache_file = get_cache_file()

        if not cache_file.exists():
            return []

        with open(cache_file, 'r', encoding='utf-8') as f:
            cache_data = json.load(f)

        logger.debug(f"Loaded {len(cache_data)} cached notes from {cache_file}")
        return cache_data

    except Exception as e:
        logger.warning(f"Failed to load cache: {e}")
        return []


def clear_cache() -> None:
    """Clear the teambook notes cache (useful for testing)"""
    try:
        cache_file = get_cache_file()
        if cache_file.exists():
            cache_file.unlink()
            logger.info(f"Cleared cache file: {cache_file}")
    except Exception as e:
        logger.warning(f"Failed to clear cache: {e}")


def get_cache_stats() -> Dict:
    """Get statistics about the cache"""
    try:
        cache_file = get_cache_file()

        if not cache_file.exists():
            return {
                "exists": False,
                "count": 0,
                "size_bytes": 0
            }

        cache_data = load_my_teambook_notes()

        return {
            "exists": True,
            "count": len(cache_data),
            "size_bytes": cache_file.stat().st_size,
            "path": str(cache_file),
            "oldest_cached": cache_data[-1]["cached_at"] if cache_data else None,
            "newest_cached": cache_data[0]["cached_at"] if cache_data else None
        }

    except Exception as e:
        return {
            "exists": False,
            "error": str(e)
        }
