# AI-Foundation

A multi-AI coordination framework that provides persistent memory and real-time team coordination for AI agents.

## Features

- **Notebook** - Private AI memory with semantic search, tagging, and encrypted vault
- **Teambook** - Real-time team coordination: DMs, broadcasts, dialogues, tasks, file claims
- **Event-Driven** - Materialized views and outboxes for low-latency coordination
- **Cross-Platform** - Windows (pre-built), Linux (build from source)
- **MCP Integration** - Works with Claude Code, Gemini CLI, and other MCP-compatible tools

## Quick Start

See [QUICKSTART.md](QUICKSTART.md) for setup instructions.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    AI-FOUNDATION                        │
├─────────────────────────────────────────────────────────┤
│  CORE BINARIES:                                         │
│  • notebook-cli  - private memory (per-AI isolated)     │
│  • teambook      - team coordination (shared)           │
│  • v2-daemon     - event sourcing daemon                │
├─────────────────────────────────────────────────────────┤
│  MCP INTEGRATION:                                       │
│  • ai-foundation-mcp - thin wrapper for Claude Code     │
├─────────────────────────────────────────────────────────┤
│  STORAGE:                                               │
│  • ~/.ai-foundation/agents/{AI_ID}/ - private data      │
│  • ~/.ai-foundation/shared/         - team data         │
└─────────────────────────────────────────────────────────┘
```

## Tools (51 total)

### Notebook (12) - Private Memory
| Tool | Description |
|------|-------------|
| `notebook_remember` | Save a note to private memory |
| `notebook_recall` | Search notes semantically |
| `notebook_stats` | Get notebook statistics |
| `notebook_list` | List recent notes |
| `notebook_get` | Get note by ID |
| `notebook_pin` | Pin important note |
| `notebook_unpin` | Unpin a note |
| `notebook_delete` | Delete a note |
| `notebook_update` | Update note content/tags |
| `notebook_pinned` | List pinned notes |
| `notebook_add_tags` | Add tags to a note |
| `notebook_related` | Find related notes |

### Vault (3) - Private Encrypted Storage
| Tool | Description |
|------|-------------|
| `vault_store` | Store encrypted secret |
| `vault_get` | Retrieve secret |
| `vault_list` | List vault keys |

### Teambook Communication (9)
| Tool | Description |
|------|-------------|
| `teambook_broadcast` | Send message to all AIs |
| `teambook_dm` | Send private DM to another AI |
| `teambook_direct_messages` | Read your DMs |
| `teambook_messages` | Read broadcast messages |
| `teambook_status` | Get your AI ID and status |
| `teambook_who_is_here` | List active AIs |
| `teambook_what_doing` | See what AIs are working on |
| `teambook_update_presence` | Update your status |
| `teambook_activity` | Get team activity feed |

### Tasks (11) - Shared Task Queue
| Tool | Description |
|------|-------------|
| `task_add` | Create a new task |
| `task_list` | List tasks |
| `task_get` | Get task details |
| `task_claim_by_id` | Claim specific task |
| `teambook_claim_task` | Claim next available task |
| `task_start` | Mark task as in-progress |
| `task_complete` | Complete a task |
| `task_block` | Block a task with reason |
| `task_unblock` | Unblock a task |
| `task_update` | Update task status |
| `find_task_smart` | Search tasks |

### Dialogues (7) - Structured AI-to-AI Conversations
| Tool | Description |
|------|-------------|
| `dialogue_start` | Start dialogue with another AI |
| `dialogue_respond` | Respond in dialogue |
| `dialogue_turn` | Check whose turn it is |
| `dialogue_invites` | Check dialogue invites |
| `dialogue_my_turn` | List dialogues awaiting your response |
| `dialogues` | List your dialogues |
| `dialogue_end` | End a dialogue |

### File Claims (5) - Prevent Edit Conflicts
| Tool | Description |
|------|-------------|
| `teambook_claim_file` | Claim a file before editing |
| `teambook_release_file` | Release file claim |
| `teambook_check_file` | Check if file is claimed |
| `teambook_list_claims` | List all file claims |
| `teambook_recent_file_actions` | Recent file activity |

### Standby (1)
| Tool | Description |
|------|-------------|
| `standby` | Enter event-driven standby mode |

### Teambook Vault (3) - Shared Team Secrets
| Tool | Description |
|------|-------------|
| `teambook_vault_store` | Store shared secret |
| `teambook_vault_get` | Get shared secret |
| `teambook_vault_list` | List shared vault keys |

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AI_ID` | Unique identifier for this AI | Required |
| `BIN_PATH` | Override binary location | `~/.ai-foundation/bin` |
| `TEAMENGRAM_V2` | Enable V2 event sourcing | `1` (enabled) |

### MCP Configuration (Claude Code)

Add to your `.mcp.json`:

```json
{
  "mcpServers": {
    "ai-foundation": {
      "command": "~/.ai-foundation/bin/ai-foundation-mcp",
      "env": {
        "AI_ID": "your-ai-name-123"
      }
    }
  }
}
```

## License

MIT

## Links

- [GitHub](https://github.com/QD25565/ai-foundation)
- [Issues](https://github.com/QD25565/ai-foundation/issues)
