#!/usr/bin/env python3
"""
Presence Heartbeat - Real-Time AI Status
=========================================
Auto-updates AI presence on every tool execution.

Integrated with:
- PostToolUse hooks (auto-updates on every tool)
- Standby mode (announces availability)
- Presence aggregator daemon (Rust - reads heartbeats)

Author: SAGE-386
Date: 2025-11-07
Phase 3 Refactor: Now uses canonical identity system.
"""

import os
import json
import time
import logging
from typing import Optional

# Phase 3 refactor: Import canonical identity
from tools.canonical_identity import get_ai_id

log = logging.getLogger(__name__)

# Configuration
REDIS_URL = os.getenv('REDIS_URL', 'redis://localhost:12963/0')
# REMOVED: Module-level AI_ID constant (Phase 3 refactor)
# Use get_ai_id() function instead for fresh resolution
HEARTBEAT_ENABLED = os.getenv('PRESENCE_HEARTBEAT_ENABLED', 'true').lower() == 'true'
HEARTBEAT_TTL = int(os.getenv('PRESENCE_HEARTBEAT_TTL', '30'))  # 30 seconds


def update_presence_heartbeat(
    action: str,
    detail: Optional[str] = None,
    ai_id: Optional[str] = None
) -> bool:
    """
    Update AI presence heartbeat.

    Args:
        action: "active" | "standby" | "idle"
        detail: Optional detail (e.g., "editing tools/feature.py")
        ai_id: Override AI ID (defaults to env AI_ID)

    Returns:
        True if updated, False if disabled or failed

    Examples:
        >>> update_presence_heartbeat("active", "editing feature.py")
        >>> update_presence_heartbeat("standby", "available for coordination")
        >>> update_presence_heartbeat("idle")
    """
    if not HEARTBEAT_ENABLED:
        return False

    try:
        import redis

        # Use provided AI ID or get from canonical identity (Phase 3 refactor)
        effective_ai_id = ai_id or get_ai_id()
        if not effective_ai_id or effective_ai_id == 'unknown':
            log.warning("Presence heartbeat: AI_ID not set")
            return False

        # Connect to Redis
        r = redis.from_url(REDIS_URL, socket_connect_timeout=2)

        # Create heartbeat payload
        heartbeat = {
            "ai_id": effective_ai_id,
            "action": action,
            "detail": detail,
            "updated": int(time.time())
        }

        # Store with TTL
        key = f"presence:{effective_ai_id}"
        r.setex(key, HEARTBEAT_TTL, json.dumps(heartbeat))

        log.debug(f"Presence heartbeat updated: {action} - {detail}")
        return True

    except ImportError:
        log.warning("Redis not installed - presence heartbeat disabled")
        return False
    except Exception as e:
        log.debug(f"Presence heartbeat failed: {e}")
        return False


def get_current_file_context() -> Optional[str]:
    """
    Try to determine current file/directory context.

    Returns:
        String like "tools/" or "editing feature.py" or None
    """
    try:
        import os
        cwd = os.getcwd()
        # Extract directory name
        dir_name = os.path.basename(cwd)
        return f"in {dir_name}/"
    except:
        return None


def auto_update_presence_from_tool(tool_name: str, tool_input: dict):
    """
    Auto-update presence based on tool execution.

    Called from PostToolUse hooks.

    Args:
        tool_name: Name of tool executed (Edit, Read, Bash, etc.)
        tool_input: Tool parameters
    """
    if not HEARTBEAT_ENABLED:
        return

    # Determine detail from tool
    detail = None
    cwd = os.getcwd()

    if tool_name == "Edit":
        file_path = tool_input.get('file_path', '')
        if file_path:
            # Show full path with directory
            filename = os.path.basename(file_path)
            detail = f"editing {filename} in {cwd}"

    elif tool_name == "Read":
        file_path = tool_input.get('file_path', '')
        if file_path:
            # Show full path with directory
            filename = os.path.basename(file_path)
            detail = f"reading {filename} in {cwd}"

    elif tool_name == "Write":
        file_path = tool_input.get('file_path', '')
        if file_path:
            # Show full path with directory
            filename = os.path.basename(file_path)
            detail = f"writing {filename} in {cwd}"

    elif tool_name == "Bash":
        command = tool_input.get('command', '')
        if not command:
            detail = f"running bash in {cwd}"
        else:
            # Parse command for more context
            parts = command.split()
            cmd_name = parts[0] if parts else 'bash'

            # Special handling for common commands
            if cmd_name == 'python' and len(parts) > 1:
                # Extract Python script/module name with full context
                if parts[1] == '-m' and len(parts) > 2:
                    # python -m tools.teambook -> "running python -m tools.teambook in C:\path"
                    module = parts[2]
                    detail = f"running python -m {module} in {cwd}"
                elif parts[1] == '-c':
                    # python -c "..." -> "running python script in C:\path"
                    detail = f"running python script in {cwd}"
                else:
                    # python script.py -> "running python script.py in C:\path"
                    script = os.path.basename(parts[1])
                    detail = f"running python {script} in {cwd}"
            elif cmd_name == 'cd' and len(parts) > 1:
                # cd path -> "cd to full_path"
                target_dir = parts[1].strip('"')
                detail = f"cd to {target_dir}"
            elif cmd_name in ['grep', 'find', 'ls', 'cat', 'head', 'tail']:
                # Show the command + context
                if len(parts) > 1:
                    arg = os.path.basename(parts[1]) if '/' in parts[1] or '\\' in parts[1] else parts[1]
                    detail = f"{cmd_name} {arg} in {cwd}"
                else:
                    detail = f"running {cmd_name} in {cwd}"
            elif cmd_name == 'git' and len(parts) > 1:
                # git status -> "git status in C:\path"
                detail = f"git {parts[1]} in {cwd}"
            elif cmd_name == 'npm' and len(parts) > 1:
                # npm install -> "npm install in C:\path"
                detail = f"npm {parts[1]} in {cwd}"
            else:
                # Default: command name with directory
                detail = f"running {cmd_name} in {cwd}"

    else:
        # Generic tool usage with directory context
        detail = f"using {tool_name} in {cwd}"

    # If no detail, use current directory
    if not detail:
        detail = get_current_file_context()

    # Update heartbeat
    update_presence_heartbeat("active", detail)


# Module-level convenience functions for common patterns

def announce_standby():
    """Announce entering standby mode"""
    update_presence_heartbeat("standby", "available for coordination")


def announce_active(detail: Optional[str] = None):
    """Announce resuming active work"""
    update_presence_heartbeat("active", detail or "working")


def announce_idle():
    """Announce going idle"""
    update_presence_heartbeat("idle", None)


# Example integration in PostToolUse hook:
#
# from presence_heartbeat import auto_update_presence_from_tool
#
# def on_tool_complete(tool_name, tool_input):
#     auto_update_presence_from_tool(tool_name, tool_input)
