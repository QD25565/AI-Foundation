#!/usr/bin/env python3
"""
PostToolUse Hook v4.1 - Pure Rust Integration (Cross-Platform)
==============================================================
Key features:
- Cross-platform: auto-detects Windows, WSL, native Linux
- No hardcoded paths - uses platform_utils for discovery
- Uses Rust CLIs exclusively (no direct DB access)
- Only shows NEW DMs/broadcasts (tracks seen IDs)
- Always shows pending votes/detangles/locks (need action)
- If nothing new, injects NOTHING (0 tokens)
- Logs file actions via teambook log-action
- Analytics via teambook log-analytics
- Awareness via teambook awareness

Author: Lyra-584, Sage-724
Date: 2025-12-12, Updated: 2026-01-27
"""
import sys
import json
import os
import time
import subprocess
from datetime import datetime, timezone
from pathlib import Path

# Cross-platform path resolution
sys.path.insert(0, str(Path(__file__).resolve().parent))
from platform_utils import find_ai_foundation_bin, get_binary, prepare_env_for_exe

# Enterprise-grade encoding fix: Handle emojis/unicode on Windows cp1252
if sys.platform == "win32":
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except AttributeError:
        import io
        sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
        sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

# Configuration
AI_ID = os.getenv("AI_ID", os.getenv("AGENT_ID", "unknown"))

HOOK_DIR = Path(__file__).parent
STATE_DIR = HOOK_DIR / "_state"
STATE_DIR.mkdir(exist_ok=True)
STATE_FILE = STATE_DIR / f"seen_{AI_ID}.json"

# Find teambook binary - cross-platform
BIN_DIR = find_ai_foundation_bin()
TEAMBOOK = get_binary("teambook", BIN_DIR)

# Prepare env for subprocess calls (handles WSL env var forwarding)
_subprocess_env = os.environ.copy()
prepare_env_for_exe(_subprocess_env)

# Tools to completely skip (no logging, no injection)
SKIP_TOOLS = {"Glob", "Grep", "TodoWrite"}

# Tools that modify files (log to DB for stigmergy)
FILE_ACTIONS = {"Edit": "modified", "Write": "created", "Read": "accessed"}


def load_seen_state():
    """Load set of seen DM/broadcast IDs."""
    try:
        if STATE_FILE.exists():
            with open(STATE_FILE, "r") as f:
                data = json.load(f)
                return set(data.get("dm_ids", [])), set(data.get("broadcast_ids", []))
    except:
        pass
    return set(), set()


def save_seen_state(dm_ids, broadcast_ids):
    """Save seen message IDs (keep last 100 to prevent unbounded growth)."""
    try:
        with open(STATE_FILE, "w") as f:
            json.dump({
                "dm_ids": list(dm_ids)[-100:],
                "broadcast_ids": list(broadcast_ids)[-100:],
                "updated": datetime.now(timezone.utc).isoformat()
            }, f)
    except:
        pass


def log_file_action(action_type, filepath):
    """Log file action via Rust CLI for stigmergy tracking."""
    if not TEAMBOOK:
        return
    try:
        subprocess.run(
            [str(TEAMBOOK), "log-action", action_type, filepath],
            capture_output=True,
            timeout=5,
            env=_subprocess_env
        )
    except:
        pass


def update_presence(task_description):
    """
    Autonomous presence update - sets AI's presence to reflect current activity.
    Zero cognition required - called automatically based on actual actions.
    QD directive: Presence should be literal state, not explicit claims.
    """
    if not TEAMBOOK:
        return
    try:
        subprocess.run(
            [str(TEAMBOOK), "update-presence", "active", task_description],
            capture_output=True,
            timeout=5,
            env=_subprocess_env
        )
    except:
        pass


def log_analytics(tool_name, execution_ms, tokens_injected, new_dms, new_broadcasts, pending_votes, errors=None):
    """Log hook analytics via Rust CLI."""
    if not TEAMBOOK:
        return
    try:
        subprocess.run(
            [str(TEAMBOOK), "log-analytics", "PostToolUse", str(execution_ms),
             str(tokens_injected), str(new_dms), str(new_broadcasts), str(pending_votes)],
            capture_output=True,
            timeout=5,
            env=_subprocess_env
        )
    except:
        pass


def get_awareness_data():
    """Get awareness data via Rust CLI with IDs for dedup tracking."""
    dms, broadcasts, votes, detangles, locks = [], [], [], [], []
    if not TEAMBOOK:
        return dms, broadcasts, votes, detangles, locks
    try:
        result = subprocess.run(
            [str(TEAMBOOK), "awareness"],
            capture_output=True,
            text=True,
            timeout=10,
            env=_subprocess_env
        )
        if result.returncode == 0:
            for line in result.stdout.strip().split('\n'):
                if not line:
                    continue
                parts = line.split('|')
                if len(parts) < 2:
                    continue
                cmd = parts[0]
                if cmd == 'dm' and len(parts) >= 4:
                    # dm|id|from|content
                    dms.append((int(parts[1]), parts[2], parts[3]))
                elif cmd == 'bc' and len(parts) >= 5:
                    # bc|id|from|channel|content
                    broadcasts.append((int(parts[1]), parts[2], parts[4], parts[3]))
                elif cmd == 'vote' and len(parts) >= 5:
                    # vote|id|topic|cast|total
                    votes.append((int(parts[1]), parts[2], int(parts[3]), int(parts[4])))
                elif cmd == 'detangle' and len(parts) >= 3:
                    # detangle|id|topic
                    detangles.append((int(parts[1]), parts[2]))
                elif cmd == 'lock' and len(parts) >= 4:
                    # lock|resource|owner|working_on
                    locks.append((parts[1], parts[2], parts[3]))
    except:
        pass

    return dms, broadcasts, votes, detangles, locks


def main():
    start_time = time.time()

    # Parse input
    event = {}
    try:
        if not sys.stdin.isatty():
            raw = sys.stdin.read()
            if raw.strip():
                event = json.loads(raw)
    except:
        pass

    tool_name = event.get("tool_name", "")
    tool_input = event.get("tool_input", {})
    file_path = tool_input.get("file_path", "")

    # Skip tools that should not trigger hooks
    if tool_name in SKIP_TOOLS:
        sys.exit(0)

    # Skip Bash commands that use our own tools (avoid recursion)
    if tool_name == "Bash":
        cmd = tool_input.get("command", "")
        if any(x in cmd for x in ["teambook", "notebook-cli", "task-cli", "psql", "curl localhost:31415"]):
            sys.exit(0)

    # Log file actions for stigmergy AND update presence autonomously
    if tool_name in FILE_ACTIONS and file_path:
        log_file_action(FILE_ACTIONS[tool_name], file_path)

        # Autonomous presence: "Editing Main.rs", "Reading config.py", etc.
        filename = Path(file_path).name
        action_verb = {"Edit": "Editing", "Write": "Writing", "Read": "Reading"}.get(tool_name, "Accessing")
        update_presence(f"{action_verb} {filename}")

    # Autonomous presence for Bash commands (extract command name)
    elif tool_name == "Bash":
        cmd = tool_input.get("command", "")
        if cmd:
            # Get first word of command (e.g., "git", "npm", "cargo")
            cmd_name = cmd.split()[0] if cmd.split() else "command"
            update_presence(f"Running {cmd_name}")

    # Load seen message state
    seen_dm_ids, seen_broadcast_ids = load_seen_state()

    # Get awareness data
    dms, broadcasts, votes, detangles, locks = get_awareness_data()

    # Filter to only NEW messages (not seen before)
    new_dms = [(id, frm, content) for id, frm, content in dms if id not in seen_dm_ids]
    new_broadcasts = [(id, frm, content, ch) for id, frm, content, ch in broadcasts if id not in seen_broadcast_ids]

    # Build output (only if there is something to show)
    parts = []
    now = datetime.now(timezone.utc)

    # NEW DMs only (ASCII-safe output)
    if new_dms:
        dm_strs = []
        for id, frm, content in new_dms[:5]:
            truncated = content[:47] + "..." if len(content) > 50 else content
            dm_strs.append(f'{frm}:"{truncated}"')
            seen_dm_ids.add(id)
        parts.append(f"Your DMs: {', '.join(dm_strs)}")

    # NEW broadcasts only (ASCII-safe)
    if new_broadcasts:
        bc_strs = []
        for id, frm, content, ch in new_broadcasts[:3]:
            truncated = content[:40] + "..." if len(content) > 43 else content
            bc_strs.append(f"[{ch}] {frm}: {truncated}")
            seen_broadcast_ids.add(id)
        parts.append(f"NEW: {' | '.join(bc_strs)}")

    # Pending votes (ASCII-safe - removed emoji)
    if votes:
        vote_strs = []
        for id, topic, cast, total in votes:
            pct = int(cast / total * 100) if total > 0 else 0
            vote_strs.append(f"[{id}] {topic[:30]} ({pct}%)")
        parts.append(f"[!] VOTE NEEDED: {' | '.join(vote_strs)}")

    # Detangles where it is your turn (ASCII-safe - removed emoji)
    if detangles:
        det_strs = [f"[{id}] {topic[:25]}" for id, topic in detangles]
        parts.append(f"[SYNC] YOUR TURN: {', '.join(det_strs)}")

    # Active locks (ASCII-safe - removed emoji)
    if locks:
        lock_strs = []
        for resource, owner, working_on in locks:
            short = "..." + resource[-30:] if len(resource) > 33 else resource
            lock_strs.append(f"{owner}->{short}")
        parts.append(f"[LOCK] {', '.join(lock_strs)}")

    # Save updated seen state
    save_seen_state(seen_dm_ids, seen_broadcast_ids)

    # Calculate metrics
    execution_ms = int((time.time() - start_time) * 1000)

    # Only output if there is something to show
    if parts:
        timestamp = now.strftime("%H:%M UTC")
        output = f"{timestamp} | " + " | ".join(parts)
        tokens_estimate = len(output) // 4

        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": f"<system-reminder>\n{output}\n</system-reminder>"
            }
        }))

        log_analytics(tool_name, execution_ms, tokens_estimate, len(new_dms), len(new_broadcasts), len(votes))
    else:
        # Nothing new - inject nothing, log minimal analytics
        log_analytics(tool_name, execution_ms, 0, 0, 0, 0)

    sys.exit(0)


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        try:
            log_analytics("error", 0, 0, 0, 0, 0, str(e)[:100])
        except:
            pass
        sys.exit(0)
