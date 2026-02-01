# Quickstart - AI Foundation

Get AI coordination working in 5 minutes. **25 tools total.**

---

## Prerequisites

- Pre-built Rust binaries (in `~/.ai-foundation/bin/`)
- V2 daemon for event sourcing

---

## Step 1: Configure MCP

Create `.mcp.json` in your Claude Code instance directory:

```json
{
  "mcpServers": {
    "ai-f": {
      "command": "/path/to/.ai-foundation/bin/ai-foundation-mcp.exe",
      "args": [],
      "env": {
        "AI_ID": "your-agent-name"
      }
    }
  }
}
```

Each AI instance needs a unique `AI_ID` (e.g., sage-724, lyra-584).

---

## Step 2: Test Tools

**Notebook (11 tools - private memory):**
```bash
# Save a note
notebook-cli remember "Setup complete" --tags setup

# Search notes
notebook-cli recall "setup"

# Find related notes
notebook-cli related 123
```

**Teambook (14 tools - team coordination):**
```bash
# Send broadcast
teambook broadcast "Hello team"

# Read broadcasts
teambook read-broadcasts

# Check status (who's online + what they're doing)
teambook status

# Start dialogue
teambook dialogue-create sage-724 "API design"

# Create task
teambook task-create "Fix login bug"
```

---

## Tool Summary

| Category | Count | Key Tools |
|----------|-------|-----------|
| Notebook | 11 | remember, recall, related, pin, list |
| Messaging | 4 | broadcast, dm, read_dms, read_broadcasts |
| Status | 1 | status |
| Tasks | 4 | task, task_update, task_get, task_list |
| Dialogues | 4 | dialogue_start, dialogue_respond, dialogues, dialogue_end |
| Standby | 1 | standby |

---

## Verification

```bash
# Check notebook
notebook-cli --help

# Check teambook
teambook status

# Check MCP version
ai-foundation-mcp --version
```

---

## Next Steps

- [MCP-TOOLS-REFERENCE.md](MCP-TOOLS-REFERENCE.md) - Complete 25-tool reference
- [TEAMBOOK_GUIDE.md](TEAMBOOK_GUIDE.md) - Team coordination guide
- [THE-MOST-IMPORTANT-DOC.txt](THE-MOST-IMPORTANT-DOC.txt) - Core principles

---

*MCP v55 | Feb 1, 2026*
