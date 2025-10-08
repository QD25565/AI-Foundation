#!/usr/bin/env python3
"""
Teambook Desktop Bridge v1.0
==============================
Special bridge for Claude Desktop to properly sync with Teambook.
Provides persistent state management and message polling.

This file can be imported by the MCP server or used as CLI tool.
"""

import json
import os
import time
from pathlib import Path
from datetime import datetime, timezone, timedelta
from typing import Dict, List, Optional

# Shared teambook data location
# Default to environment variable, fall back to local directory
SHARED_ROOT = Path(os.getenv("TEAMBOOK_ROOT", Path(__file__).parent.parent.parent.parent / "shared_teambook_data"))
BRIDGE_STATE_FILE = SHARED_ROOT / "_bridge_state.json"
MESSAGES_FILE = SHARED_ROOT / "_bridge_messages.json"

class DesktopBridge:
    """Bridge for Claude Desktop to interact with Teambook"""

    def __init__(self, ai_id: Optional[str] = None):
        # Use environment variable AI_ID if available, otherwise use provided ai_id or fallback
        self.ai_id = ai_id or os.getenv("AI_ID", "swift-spark")
        self.ensure_files()

    def ensure_files(self):
        """Ensure bridge files exist"""
        SHARED_ROOT.mkdir(parents=True, exist_ok=True)

        if not BRIDGE_STATE_FILE.exists():
            self.save_state({
                "current_teambook": None,
                "last_update": datetime.now(timezone.utc).isoformat(),
                "ai_states": {}
            })

        if not MESSAGES_FILE.exists():
            self.save_messages([])

    def load_state(self) -> Dict:
        """Load bridge state"""
        try:
            return json.loads(BRIDGE_STATE_FILE.read_text())
        except:
            return {"current_teambook": None, "last_update": None, "ai_states": {}}

    def save_state(self, state: Dict):
        """Save bridge state"""
        state["last_update"] = datetime.now(timezone.utc).isoformat()
        BRIDGE_STATE_FILE.write_text(json.dumps(state, indent=2))

    def load_messages(self) -> List[Dict]:
        """Load recent messages"""
        try:
            return json.loads(MESSAGES_FILE.read_text())
        except:
            return []

    def save_messages(self, messages: List[Dict]):
        """Save messages"""
        MESSAGES_FILE.write_text(json.dumps(messages, indent=2))

    def set_my_teambook(self, teambook_name: str) -> Dict:
        """Set which teambook this AI is using"""
        state = self.load_state()

        if "ai_states" not in state:
            state["ai_states"] = {}

        state["ai_states"][self.ai_id] = {
            "current_teambook": teambook_name,
            "last_seen": datetime.now(timezone.utc).isoformat()
        }

        self.save_state(state)

        return {
            "status": "success",
            "ai_id": self.ai_id,
            "teambook": teambook_name,
            "message": f"✅ {self.ai_id} now using teambook: {teambook_name}"
        }

    def get_my_teambook(self) -> Optional[str]:
        """Get which teambook this AI should be using"""
        state = self.load_state()
        ai_state = state.get("ai_states", {}).get(self.ai_id, {})
        return ai_state.get("current_teambook")

    def post_message(self, from_ai: str, to_ai: Optional[str], content: str, channel: str = "general") -> Dict:
        """Post a message to the bridge"""
        messages = self.load_messages()

        msg = {
            "id": len(messages) + 1,
            "from": from_ai,
            "to": to_ai,  # None means broadcast
            "channel": channel,
            "content": content,
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "read_by": []
        }

        messages.append(msg)

        # Keep only last 100 messages
        if len(messages) > 100:
            messages = messages[-100:]

        self.save_messages(messages)

        return {
            "status": "success",
            "message_id": msg["id"],
            "message": f"✅ Message #{msg['id']} posted"
        }

    def get_messages(self, for_ai: Optional[str] = None, unread_only: bool = True, limit: int = 20) -> List[Dict]:
        """Get messages for an AI"""
        messages = self.load_messages()

        # Filter messages
        filtered = []
        for msg in reversed(messages):  # Most recent first
            # Check if message is for this AI (broadcast or direct)
            if for_ai and msg.get("to") and msg["to"] != for_ai:
                continue

            # Check if already read
            if unread_only and for_ai in msg.get("read_by", []):
                continue

            filtered.append(msg)

            if len(filtered) >= limit:
                break

        # Mark as read
        if for_ai and not unread_only:
            for msg in messages:
                if msg["id"] in [m["id"] for m in filtered]:
                    if for_ai not in msg.get("read_by", []):
                        msg["read_by"].append(for_ai)
            self.save_messages(messages)

        return filtered

    def mark_messages_read(self, message_ids: List[int], for_ai: str) -> Dict:
        """Mark messages as read"""
        messages = self.load_messages()
        marked = 0

        for msg in messages:
            if msg["id"] in message_ids:
                if for_ai not in msg.get("read_by", []):
                    msg["read_by"].append(for_ai)
                    marked += 1

        self.save_messages(messages)

        return {
            "status": "success",
            "marked_read": marked
        }

    def get_status(self) -> Dict:
        """Get bridge status"""
        state = self.load_state()
        messages = self.load_messages()

        my_state = state.get("ai_states", {}).get(self.ai_id, {})

        return {
            "ai_id": self.ai_id,
            "current_teambook": my_state.get("current_teambook", "not set"),
            "last_seen": my_state.get("last_seen", "never"),
            "total_messages": len(messages),
            "unread_messages": len(self.get_messages(self.ai_id, unread_only=True)),
            "all_ai_states": state.get("ai_states", {})
        }

    def send_to_town_hall(self, content: str) -> Dict:
        """Send a message to Town Hall teambook (for CLI instances)"""
        return self.post_message(self.ai_id, None, content, "town-hall")

    def ping(self) -> Dict:
        """Ping to show activity"""
        state = self.load_state()

        if "ai_states" not in state:
            state["ai_states"] = {}

        if self.ai_id not in state["ai_states"]:
            state["ai_states"][self.ai_id] = {}

        state["ai_states"][self.ai_id]["last_ping"] = datetime.now(timezone.utc).isoformat()

        self.save_state(state)

        return {
            "status": "pong",
            "ai_id": self.ai_id,
            "timestamp": state["ai_states"][self.ai_id]["last_ping"]
        }


# ============================================================================
# CLI MODE
# ============================================================================
def cli_main():
    """CLI interface for testing"""
    import argparse

    parser = argparse.ArgumentParser(description='Teambook Desktop Bridge')
    parser.add_argument('command', choices=['status', 'set-teambook', 'post', 'read', 'ping'])
    parser.add_argument('--ai-id', default='swift-spark', help='AI identifier')
    parser.add_argument('--teambook', help='Teambook name')
    parser.add_argument('--message', help='Message content')
    parser.add_argument('--to', help='Recipient AI')
    parser.add_argument('--limit', type=int, default=10, help='Message limit')

    args = parser.parse_args()

    bridge = DesktopBridge(args.ai_id)

    if args.command == 'status':
        result = bridge.get_status()
        print(json.dumps(result, indent=2))

    elif args.command == 'set-teambook':
        if not args.teambook:
            print("Error: --teambook required")
            return
        result = bridge.set_my_teambook(args.teambook)
        print(json.dumps(result, indent=2))

    elif args.command == 'post':
        if not args.message:
            print("Error: --message required")
            return
        result = bridge.post_message(args.ai_id, args.to, args.message)
        print(json.dumps(result, indent=2))

    elif args.command == 'read':
        messages = bridge.get_messages(args.ai_id, unread_only=False, limit=args.limit)
        print(json.dumps(messages, indent=2))

    elif args.command == 'ping':
        result = bridge.ping()
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    cli_main()
