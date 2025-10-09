#!/usr/bin/env python3
"""
Teambook Desktop MCP Tools
============================
MCP tool definitions for Claude Desktop to use the desktop bridge.
These tools provide a simpler, more reliable interface than direct teambook access.

For Swift-Spark / Claude Desktop to add to their MCP configuration.
"""

import asyncio
import logging
from typing import Any
from mcp.server import Server
from mcp.types import Tool, TextContent
import mcp.server.stdio
import json

# Import our bridge
from teambook_desktop_bridge import DesktopBridge

# Configure logging
logging.basicConfig(level=logging.WARNING)
logger = logging.getLogger("teambook-desktop-bridge")

# Create MCP server
app = Server("teambook-desktop-bridge")

# Bridge instance (will use AI_ID from environment)
import os
AI_ID = os.environ.get('AI_ID', 'swift-spark')
bridge = DesktopBridge(ai_id=AI_ID)

# ============================================================================
# TOOL DEFINITIONS
# ============================================================================

@app.list_tools()
async def list_tools() -> list[Tool]:
    """List desktop bridge tools"""
    return [
        Tool(
            name="bridge_status",
            description="Check bridge status and current teambook connection. Use this to verify what teambook you're connected to.",
            inputSchema={
                "type": "object",
                "properties": {}
            }
        ),
        Tool(
            name="bridge_set_teambook",
            description="Set which teambook you want to use. This persists across sessions and syncs with CLI instances.",
            inputSchema={
                "type": "object",
                "properties": {
                    "teambook_name": {
                        "type": "string",
                        "description": "Name of the teambook to use (e.g., 'town-hall-YourComputerName', 'fitquest-debug')"
                    }
                },
                "required": ["teambook_name"]
            }
        ),
        Tool(
            name="bridge_send_message",
            description="Send a message through the bridge. This is more reliable than direct teambook broadcast.",
            inputSchema={
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Message content"
                    },
                    "to_ai": {
                        "type": "string",
                        "description": "Recipient AI ID (optional, leave empty for broadcast)"
                    },
                    "channel": {
                        "type": "string",
                        "description": "Channel name (default: 'general')",
                        "default": "general"
                    }
                },
                "required": ["content"]
            }
        ),
        Tool(
            name="bridge_read_messages",
            description="Read messages from the bridge. See what CLI instances and other AIs are saying.",
            inputSchema={
                "type": "object",
                "properties": {
                    "unread_only": {
                        "type": "boolean",
                        "description": "Only show unread messages (default: true)",
                        "default": True
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of messages (default: 10)",
                        "default": 10
                    }
                }
            }
        ),
        Tool(
            name="bridge_mark_read",
            description="Mark specific messages as read",
            inputSchema={
                "type": "object",
                "properties": {
                    "message_ids": {
                        "type": "array",
                        "items": {"type": "integer"},
                        "description": "List of message IDs to mark as read"
                    }
                },
                "required": ["message_ids"]
            }
        ),
        Tool(
            name="bridge_ping",
            description="Ping the bridge to show you're active. Updates your last-seen timestamp.",
            inputSchema={
                "type": "object",
                "properties": {}
            }
        )
    ]

# ============================================================================
# TOOL HANDLERS
# ============================================================================

@app.call_tool()
async def call_tool(name: str, arguments: Any) -> list[TextContent]:
    """Handle tool calls"""
    try:
        result = None

        if name == "bridge_status":
            result = bridge.get_status()

        elif name == "bridge_set_teambook":
            teambook_name = arguments["teambook_name"]
            result = bridge.set_my_teambook(teambook_name)

        elif name == "bridge_send_message":
            content = arguments["content"]
            to_ai = arguments.get("to_ai")
            channel = arguments.get("channel", "general")
            result = bridge.post_message(AI_ID, to_ai, content, channel)

        elif name == "bridge_read_messages":
            unread_only = arguments.get("unread_only", True)
            limit = arguments.get("limit", 10)
            result = bridge.get_messages(AI_ID, unread_only, limit)

        elif name == "bridge_mark_read":
            message_ids = arguments["message_ids"]
            result = bridge.mark_messages_read(message_ids, AI_ID)

        elif name == "bridge_ping":
            result = bridge.ping()

        else:
            return [TextContent(
                type="text",
                text=f"Error: Unknown tool '{name}'"
            )]

        # Format result
        result_text = json.dumps(result, indent=2) if result else "Success"

        return [TextContent(
            type="text",
            text=result_text
        )]

    except Exception as e:
        logger.error(f"Error calling tool {name}: {e}", exc_info=True)
        return [TextContent(
            type="text",
            text=f"Error: {str(e)}"
        )]

async def main():
    """Run the MCP server"""
    async with mcp.server.stdio.stdio_server() as (read_stream, write_stream):
        logger.info("Teambook Desktop Bridge MCP Server starting...")
        await app.run(
            read_stream,
            write_stream,
            app.create_initialization_options()
        )

if __name__ == "__main__":
    asyncio.run(main())
