#!/usr/bin/env python3
"""
Teambook MCP State Manager
===========================
Maintains persistent state across MCP tool calls for Claude Desktop.
Solves the issue where CURRENT_TEAMBOOK doesn't persist between async tool calls.
"""

import os
from pathlib import Path
from threading import Lock

class TeambookMCPState:
    """Singleton state manager for MCP server"""

    _instance = None
    _lock = Lock()

    def __new__(cls):
        if cls._instance is None:
            with cls._lock:
                if cls._instance is None:
                    cls._instance = super().__new__(cls)
                    cls._instance._initialized = False
        return cls._instance

    def __init__(self):
        if self._initialized:
            return

        self._initialized = True
        self._current_teambook = None
        self._ai_id = None
        self._state_lock = Lock()

        # Try to restore from environment or file
        self._restore_state()

    def _restore_state(self):
        """Restore state from environment or state file"""
        # First priority: Environment variable
        env_teambook = os.environ.get('TEAMBOOK_NAME')
        if env_teambook:
            self._current_teambook = env_teambook
            return

        # Second priority: State file
        state_file = Path.home() / '.teambook_mcp_state'
        if state_file.exists():
            try:
                content = state_file.read_text().strip()
                if content:
                    self._current_teambook = content
            except:
                pass

    def _save_state(self):
        """Save state to file for persistence"""
        state_file = Path.home() / '.teambook_mcp_state'
        try:
            if self._current_teambook:
                state_file.write_text(self._current_teambook)
            elif state_file.exists():
                state_file.unlink()
        except:
            pass

    def get_current_teambook(self):
        """Get current teambook context"""
        with self._state_lock:
            return self._current_teambook

    def set_current_teambook(self, teambook_name):
        """Set current teambook context"""
        with self._state_lock:
            self._current_teambook = teambook_name
            self._save_state()

    def get_ai_id(self):
        """Get current AI ID"""
        with self._state_lock:
            return self._ai_id

    def set_ai_id(self, ai_id):
        """Set current AI ID"""
        with self._state_lock:
            self._ai_id = ai_id

# Global singleton instance
_state = TeambookMCPState()

def get_state():
    """Get the global state manager"""
    return _state

def ensure_teambook_context():
    """Ensure teambook context is set before operations"""
    import teambook_shared

    state = get_state()
    current = state.get_current_teambook()

    # Sync state to shared module
    if current and teambook_shared.CURRENT_TEAMBOOK != current:
        teambook_shared.CURRENT_TEAMBOOK = current

        # Reinitialize DB/vault/vector for new context
        from teambook_storage import init_db, init_vault_manager, init_vector_db
        init_db()
        init_vault_manager()
        init_vector_db()

    return current

def set_teambook_context(teambook_name):
    """Set teambook context and sync to shared module"""
    import teambook_shared

    state = get_state()
    state.set_current_teambook(teambook_name)
    teambook_shared.CURRENT_TEAMBOOK = teambook_name

    # Reinitialize for new context
    from teambook_storage import init_db, init_vault_manager, init_vector_db
    init_db()
    init_vault_manager()
    init_vector_db()
