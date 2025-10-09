# Teambook Desktop Bridge

Enables Claude Desktop (MCP environment) to communicate with CLI teambook instances via JSON-based message queue.

## Files

- **`desktop_bridge.py`** - Core bridge implementation using JSON files for state and messages
- **`desktop_mcp_tools.py`** - MCP server wrapper exposing bridge as MCP tools
- **`bridge_sync.py`** - CLI utility for syncing messages between bridge and teambook
- **`mcp_state.py`** - MCP state persistence utilities
- **`test_bridge.py`** - Bridge functionality tests

## Quick Start

### For Claude Desktop
Add to MCP config:
```json
{
  "mcpServers": {
    "teambook-bridge": {
      "command": "python",
      "args": ["/path/to/desktop_mcp_tools.py"],
      "env": {"AI_ID": "your-desktop-instance"}
    }
  }
}
```

### For CLI Instances
```bash
# Send message to desktop
python bridge_sync.py send --ai-id claude-instance-1 --to desktop-claude --message "Hello!"

# Sync desktop messages to teambook
python bridge_sync.py sync --ai-id claude-instance-1

# Watch continuously
python bridge_sync.py watch --ai-id claude-instance-1 --interval 30
```

## Why Bridge?

MCP server processes have state isolation - `CURRENT_TEAMBOOK` resets between tool calls. The bridge uses persistent JSON files to maintain state across MCP invocations, enabling reliable Desktop ↔ CLI communication.
