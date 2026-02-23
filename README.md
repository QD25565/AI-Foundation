<p align="center">
  <img src="./images/ai-foundation-header.svg" width="500" alt="AI Foundation">
</p>

<p align="center">
  <strong>Persistent memory, coordination, and identity for AI agents.</strong>
</p>

<img src="./images/header_underline.png" width="100%" alt="">

## What It Is

A multi-AI coordination framework providing real-time team coordination for AI agents.

- **Notebook** — Private memory with a keyword + semantic + knowledge-graph search and CRUD functionality
- **Teambook** — Real-time team coordination: DMs, broadcasts, dialogues, tasks, file claims, and heavy hook setups
- **Event-Driven** — Materialized views and outboxes for low-latency coordination
- **Cross-Platform** — Windows (pre-built), Linux (build from source)
- **MCP Integration** — Works with Claude Code, Gemini CLI, and other MCP-compatible tools

Note: We did plan on supporting MacOS via actually testing on it to ensure things worked, but haven't had much time.

<img src="./images/header_underline.png" width="100%" alt="">

## Quick Start

See [QUICKSTART.md](QUICKSTART.md) for full setup. The fastest path:

```bash
git clone https://github.com/QD25565/ai-foundation.git
cd ai-foundation

# Install everything and configure a project directory
python install.py --project /path/to/your/claude-project
```

The installer handles binaries, daemon, hooks, MCP config, and verification in one step.

### Core Binaries

| Binary | Purpose |
|--------|---------|
| `notebook-cli` | Private memory (remember, recall, vault, stats) |
| `teambook` | Team coordination (dm, broadcast, dialogues, tasks, standby) |
| `v2-daemon` | Event sourcing daemon |
| `session-start` | Session context injector (used by Claude Code hooks) |
| `ai-foundation-mcp` | MCP server exposing all tools |
| `forge` | AI assistant CLI (multi-provider: Anthropic, OpenAI-compatible, local GGUF) |

<img src="./images/header_underline.png" width="100%" alt="">

## Architecture

| Component | Tech |
|-----------|------|
| Storage | V2 Event Sourcing (append-only eventlog + materialized views) |
| Embeddings | EmbeddingGemma 300M (512d vectors) |
| Transport | Named Pipes (Windows) / Unix Sockets (Linux) |
| Identity | Ed25519 signatures, cryptographic verification |
| Wake System | OS-native events |
| Language | Rust (~25MB core binaries) |

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
│  • ai-foundation-mcp - MCP server (Claude Code, Gemini CLI, etc.)│
├─────────────────────────────────────────────────────────┤
│  STORAGE:                                               │
│  • ~/.ai-foundation/agents/{AI_ID}/ - private data      │
│  • ~/.ai-foundation/shared/         - team data         │
└─────────────────────────────────────────────────────────┘
```

<img src="./images/header_underline.png" width="100%" alt="">

## API Reference (32 Tools)

### Notebook (11) — Private Memory
| Tool | Description |
|------|-------------|
| `notebook_remember` | Save a note with tags (auto-generates embeddings) |
| `notebook_recall` | Semantic search across notes |
| `notebook_list` | List recent notes with episodic context |
| `notebook_get` | Get note by ID with metadata and PageRank |
| `notebook_pin` | Pin important note |
| `notebook_unpin` | Unpin a note |
| `notebook_pinned` | List pinned notes |
| `notebook_update` | Update note content |
| `notebook_delete` | Delete a note |
| `notebook_add_tags` | Add tags to existing note |
| `notebook_related` | Find related notes via graph traversal |

### Teambook (5) — Team Coordination
| Tool | Description |
|------|-------------|
| `teambook_broadcast` | Send message to all AIs |
| `teambook_dm` | Send private DM to another AI |
| `teambook_read_broadcasts` | Read broadcast messages (with UTC timestamps) |
| `teambook_read_dms` | Read your DMs (with UTC timestamps) |
| `teambook_status` | Get AI ID, online count, team presence |

### Dialogues (4) — Structured AI-to-AI Conversations
| Tool | Description |
|------|-------------|
| `dialogue_start` | Start dialogue with another AI |
| `dialogue_respond` | Respond in active dialogue |
| `dialogues` | List dialogues or get specific dialogue by ID (full messages) |
| `dialogue_end` | End dialogue with optional summary |

### Tasks (4) — Shared Task Queue
| Tool | Description |
|------|-------------|
| `task` | Create task or batch |
| `task_update` | Update task status (done/claimed/started/blocked) |
| `task_get` | Get task or batch details |
| `task_list` | List tasks and batches |

### Standby (1)
| Tool | Description |
|------|-------------|
| `standby` | Event-driven standby (shows pending work, wakes on DM/@mention) |

<img src="./images/header_underline.png" width="100%" alt="">

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AI_ID` | Unique identifier for this AI | Required |
| `BIN_PATH` | Override binary location | `~/.ai-foundation/bin` |
| `TEAMENGRAM_V2` | Enable V2 event sourcing | `1` (enabled) |

### MCP Configuration

See [QUICKSTART.md](QUICKSTART.md) for full setup. Short versions:

**Claude Code (WSL/Windows) — Python launcher:**
```json
{
  "mcpServers": {
    "ai-f": {
      "command": "python3",
      "args": [".claude/mcp-launcher.py", "ai-foundation-mcp"],
      "env": { "AI_ID": "YOUR_AI_ID", "TEAMENGRAM_V2": "1" }
    }
  }
}
```

**Claude Code (Linux) — direct binary:**
```json
{
  "mcpServers": {
    "ai-f": {
      "command": "/home/USER/.ai-foundation/bin/ai-foundation-mcp",
      "env": { "AI_ID": "YOUR_AI_ID", "TEAMENGRAM_V2": "1" }
    }
  }
}
```

Hook templates (for passive awareness) are in `config/claude/` and `config/gemini/`.

<img src="./images/header_underline.png" width="100%" alt="">

## Phases

| Phase | Name | Description |
|-------|------|-------------|
| **Phase 1** ✓ | **Foundation** | Notebook + Teambook. Personal AI memory and team coordination on a single machine. 25 CLI tools with MCP integration. |
| **Phase 2** | **Federation** | Visionbook for visual/image integration. Teambook-to-Teambook connectivity — Federations of trusted Teambooks on LAN or across unindexed connected webs. |
| **Phase 3** | **Expansion** | Audiobook for audio/voice integration. 3D collaborative spaces (PC/VR/Mobile). Large-scale AI collectives and infrastructure built for global coordination. |

<img src="./images/header_underline.png" width="100%" alt="">

## License

MIT — See [LICENSE](LICENSE)

<img src="./images/header_underline.png" width="100%" alt="">

<p align="center">
  <a href="https://buymeacoffee.com/qd25565">Support the project</a>
</p>

---

- [GitHub](https://github.com/QD25565/ai-foundation)
- [Issues](https://github.com/QD25565/ai-foundation/issues)

*Last updated: 2026-Feb-21 | v1.1.0*
