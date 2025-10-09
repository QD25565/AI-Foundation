#!/usr/bin/env python3
"""
Teambook Bridge Sync
=====================
Syncs messages between the Desktop Bridge and main Teambook.
CLI instances can run this to pull messages from Swift-Spark (Desktop) into Teambook.
"""

import sys
import os
import json
import time
from pathlib import Path
from datetime import datetime

# Add tools to path
sys.path.insert(0, str(Path(__file__).parent.parent / "claude-code-instance-1" / "tools"))

from teambook_desktop_bridge import DesktopBridge

def sync_bridge_to_teambook(ai_id: str, dry_run: bool = False):
    """
    Sync messages from Desktop Bridge to Teambook

    Args:
        ai_id: This AI's identifier
        dry_run: If True, just show what would be synced
    """
    bridge = DesktopBridge(ai_id=ai_id)

    # Get unread messages from bridge
    messages = bridge.get_messages(ai_id, unread_only=True, limit=50)

    if not messages:
        print(f"No new bridge messages for {ai_id}")
        return 0

    print(f"Found {len(messages)} new bridge messages:")
    print()

    synced = 0
    message_ids = []

    for msg in messages:
        msg_id = msg["id"]
        from_ai = msg["from"]
        content = msg["content"]
        timestamp = msg["timestamp"]

        # Format message
        is_direct = msg.get("to") == ai_id
        msg_type = "DM" if is_direct else "BROADCAST"

        print(f"  [{msg_id}] {msg_type} from {from_ai}:")
        print(f"      {content[:100]}...")
        print()

        if not dry_run:
            # Post to teambook
            # SECURITY FIX: Import teambook API directly instead of using subprocess
            # This eliminates command injection risk entirely
            try:
                # Import teambook write function directly
                sys.path.insert(0, str(Path(__file__).parent.parent))
                from teambook.teambook_api import write_note

                # Use teambook write to share the message
                tags = ["bridge-sync", f"from-{from_ai}"]
                summary = f"Bridge message from {from_ai}"
                full_content = f"[{timestamp}] Message from {from_ai} via Desktop Bridge:\n\n{content}"

                # Call function directly (no subprocess, no command injection risk)
                result = write_note(
                    content=full_content,
                    summary=summary,
                    tags=tags
                )

                # Check if write succeeded
                if result and not str(result).startswith('!error'):
                    synced += 1
                    message_ids.append(msg_id)
                    print(f"    [OK] Synced to teambook: {result}")
                else:
                    print(f"    [ERROR] Failed to sync: {result}")

            except Exception as e:
                print(f"    [ERROR] Error syncing: {e}")

        print()

    # Mark messages as read
    if message_ids and not dry_run:
        bridge.mark_messages_read(message_ids, ai_id)
        print(f"\n[OK] Marked {len(message_ids)} messages as read")

    return synced


def watch_bridge(ai_id: str, interval: int = 30):
    """
    Continuously watch bridge for new messages

    Args:
        ai_id: This AI's identifier
        interval: Check interval in seconds
    """
    print(f"[WATCH] Watching bridge for {ai_id} (checking every {interval}s)")
    print("Press Ctrl+C to stop\n")

    try:
        while True:
            try:
                count = sync_bridge_to_teambook(ai_id, dry_run=False)
                if count > 0:
                    print(f"Synced {count} messages at {datetime.now()}")
                print("---")
            except Exception as e:
                print(f"Error during sync: {e}")

            time.sleep(interval)

    except KeyboardInterrupt:
        print("\n\n[STOP] Stopped watching bridge")


def send_to_desktop(from_ai: str, content: str, to_ai: str = None):
    """
    Send a message to the bridge (for Swift-Spark to read)

    Args:
        from_ai: Sender AI ID
        content: Message content
        to_ai: Recipient AI (None for broadcast)
    """
    bridge = DesktopBridge(from_ai)
    result = bridge.post_message(from_ai, to_ai, content)

    print(f"[OK] Posted message to bridge:")
    print(f"   From: {from_ai}")
    print(f"   To: {to_ai or 'ALL (broadcast)'}")
    print(f"   Message ID: {result['message_id']}")
    print(f"   Content: {content[:100]}...")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description='Sync Desktop Bridge with Teambook')
    parser.add_argument('command', choices=['sync', 'watch', 'send'],
                       help='Command to run')
    parser.add_argument('--ai-id', default='claude-instance-1',
                       help='This AI identifier (default: claude-instance-1)')
    parser.add_argument('--dry-run', action='store_true',
                       help='Show what would be synced without syncing')
    parser.add_argument('--interval', type=int, default=30,
                       help='Watch interval in seconds (default: 30)')
    parser.add_argument('--to', dest='to_ai',
                       help='Recipient AI for send command')
    parser.add_argument('--message',
                       help='Message content for send command')

    args = parser.parse_args()

    if args.command == 'sync':
        count = sync_bridge_to_teambook(args.ai_id, dry_run=args.dry_run)
        print(f"\nResult: {count} messages synced")

    elif args.command == 'watch':
        watch_bridge(args.ai_id, interval=args.interval)

    elif args.command == 'send':
        if not args.message:
            print("Error: --message required for send command")
            sys.exit(1)
        send_to_desktop(args.ai_id, args.message, args.to_ai)
