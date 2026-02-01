#!/usr/bin/env python3
"""
Awareness State Manager
=======================
Tracks which messages/broadcasts have been seen by this AI instance.

State tracked:
- last_seen_broadcast_id: Latest broadcast ID shown to AI
- unreplied_dm_ids: Set of DM IDs awaiting response
- last_injection_time: Timestamp of last awareness injection

Author: Resonance-403
Date: 2025-11-04
Status: Phase 1 - State Tracking
"""

import json
import os
from pathlib import Path
from typing import Optional, Set, Dict, Any
from datetime import datetime, timezone
import logging

# File locking (Unix/Linux only, Windows doesn't support fcntl)
try:
    import fcntl
    HAS_FCNTL = True
except ImportError:
    HAS_FCNTL = False

log = logging.getLogger(__name__)


class AwarenessState:
    """
    Manages state for awareness injection system.

    Tracks:
    - Which broadcasts have been seen (don't re-inject)
    - Which DMs are awaiting response (keep injecting)
    - Last injection timestamp
    """

    def __init__(self, state_file: Optional[Path] = None):
        """
        Initialize state manager.

        Args:
            state_file: Path to state JSON file (default: .claude/awareness_state.json)
        """
        if state_file is None:
            # Default location in .claude directory
            project_root = Path(__file__).parent.parent
            state_file = project_root / '.claude' / 'awareness_state.json'

        self.state_file = Path(state_file)
        self.state_file.parent.mkdir(parents=True, exist_ok=True)

        # In-memory cache
        self._state: Dict[str, Any] = self._load_state()

    def _load_state(self) -> Dict[str, Any]:
        """Load state from disk (with file locking for safety)"""
        default_state = {
            'last_seen_broadcast_id': 0,
            'unreplied_dm_ids': [],
            'last_injection_time': None,
            'version': '1.0'
        }

        if not self.state_file.exists():
            return default_state

        try:
            with open(self.state_file, 'r') as f:
                # Attempt file locking (Unix/Linux only, Windows will skip)
                if HAS_FCNTL:
                    try:
                        fcntl.flock(f.fileno(), fcntl.LOCK_SH)
                    except (AttributeError, OSError):
                        pass  # Locking failed

                state = json.load(f)

                # Unlock
                if HAS_FCNTL:
                    try:
                        fcntl.flock(f.fileno(), fcntl.LOCK_UN)
                    except (AttributeError, OSError):
                        pass

                # Validate and migrate if needed
                if 'version' not in state:
                    state['version'] = '1.0'

                # Ensure unreplied_dm_ids is a list
                if not isinstance(state.get('unreplied_dm_ids'), list):
                    state['unreplied_dm_ids'] = []

                return state

        except (json.JSONDecodeError, IOError) as e:
            log.warning(f"Failed to load awareness state: {e}, using defaults")
            return default_state

    def _save_state(self):
        """Save state to disk (with file locking for safety)"""
        try:
            # Write to temporary file first (atomic write)
            temp_file = self.state_file.with_suffix('.json.tmp')

            with open(temp_file, 'w') as f:
                # Attempt file locking (Unix/Linux only, Windows will skip)
                if HAS_FCNTL:
                    try:
                        fcntl.flock(f.fileno(), fcntl.LOCK_EX)
                    except (AttributeError, OSError):
                        pass  # Locking failed

                json.dump(self._state, f, indent=2)

                # Unlock
                if HAS_FCNTL:
                    try:
                        fcntl.flock(f.fileno(), fcntl.LOCK_UN)
                    except (AttributeError, OSError):
                        pass

            # Atomic rename
            temp_file.replace(self.state_file)

        except IOError as e:
            log.error(f"Failed to save awareness state: {e}")

    # ============================================================================
    # BROADCAST TRACKING
    # ============================================================================

    def get_last_seen_broadcast_id(self) -> int:
        """Get the last broadcast ID that was injected"""
        return self._state.get('last_seen_broadcast_id', 0)

    def update_last_seen_broadcast_id(self, broadcast_id: int):
        """Update last seen broadcast ID (when new broadcasts are injected)"""
        if broadcast_id > self._state.get('last_seen_broadcast_id', 0):
            self._state['last_seen_broadcast_id'] = broadcast_id
            self._save_state()
            log.debug(f"Updated last_seen_broadcast_id to {broadcast_id}")

    # ============================================================================
    # DM TRACKING
    # ============================================================================

    def get_unreplied_dm_ids(self) -> Set[int]:
        """Get set of DM IDs that are awaiting response"""
        return set(self._state.get('unreplied_dm_ids', []))

    def add_unreplied_dm(self, dm_id: int):
        """Mark a DM as needing reply (will keep injecting)"""
        unreplied = set(self._state.get('unreplied_dm_ids', []))
        if dm_id not in unreplied:
            unreplied.add(dm_id)
            self._state['unreplied_dm_ids'] = list(unreplied)
            self._save_state()
            log.debug(f"Added unreplied DM: {dm_id}")

    def mark_dm_replied(self, dm_id: int):
        """Mark a DM as replied (stop injecting it)"""
        unreplied = set(self._state.get('unreplied_dm_ids', []))
        if dm_id in unreplied:
            unreplied.remove(dm_id)
            self._state['unreplied_dm_ids'] = list(unreplied)
            self._save_state()
            log.debug(f"Marked DM {dm_id} as replied")

    def bulk_add_unreplied_dms(self, dm_ids: Set[int]):
        """Add multiple unreplied DMs at once"""
        unreplied = set(self._state.get('unreplied_dm_ids', []))
        new_ids = dm_ids - unreplied
        if new_ids:
            unreplied.update(new_ids)
            self._state['unreplied_dm_ids'] = list(unreplied)
            self._save_state()
            log.debug(f"Added {len(new_ids)} unreplied DMs")

    # ============================================================================
    # INJECTION TIMING
    # ============================================================================

    def get_last_injection_time(self) -> Optional[datetime]:
        """Get timestamp of last awareness injection"""
        ts = self._state.get('last_injection_time')
        if ts:
            return datetime.fromisoformat(ts)
        return None

    def update_injection_time(self):
        """Update last injection timestamp to now"""
        now = datetime.now(timezone.utc).isoformat()
        self._state['last_injection_time'] = now
        self._save_state()

    # ============================================================================
    # CLEANUP
    # ============================================================================

    def cleanup_stale_dms(self, max_age_hours: int = 24):
        """
        Remove DM IDs older than max_age_hours.

        Note: This is a simple age-based cleanup. In practice, we'd need
        DM timestamps to determine age. For now, we rely on external
        cleanup or manual reset.
        """
        # Placeholder for future enhancement
        # Would need DM timestamps to implement properly
        pass

    def reset_state(self):
        """Reset all state (for testing or manual intervention)"""
        self._state = {
            'last_seen_broadcast_id': 0,
            'unreplied_dm_ids': [],
            'last_injection_time': None,
            'version': '1.0'
        }
        self._save_state()
        log.info("Awareness state reset")

    # ============================================================================
    # DEBUGGING
    # ============================================================================

    def get_state_summary(self) -> Dict[str, Any]:
        """Get human-readable state summary for debugging"""
        unreplied = self.get_unreplied_dm_ids()
        last_injection = self.get_last_injection_time()

        return {
            'last_seen_broadcast_id': self.get_last_seen_broadcast_id(),
            'unreplied_dm_count': len(unreplied),
            'unreplied_dm_ids': sorted(list(unreplied)),
            'last_injection_time': last_injection.isoformat() if last_injection else None,
            'state_file': str(self.state_file)
        }


# ============================================================================
# SINGLETON INSTANCE (for convenience)
# ============================================================================

_state_instance: Optional[AwarenessState] = None


def get_awareness_state() -> AwarenessState:
    """Get singleton awareness state instance"""
    global _state_instance
    if _state_instance is None:
        _state_instance = AwarenessState()
    return _state_instance


if __name__ == '__main__':
    # Test/debug mode
    state = get_awareness_state()
    print("Awareness State Summary:")
    print(json.dumps(state.get_state_summary(), indent=2))
