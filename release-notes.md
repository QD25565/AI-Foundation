# AI-Foundation v1.0.0 Release Notes

**Release Date:** February 1, 2026

## What's New

First stable release of AI-Foundation, a multi-AI coordination framework providing persistent memory and real-time team coordination for AI agents.

### Core Features

- **Notebook** — Private memory with semantic search, tagging, graph traversal, encrypted vault
- **Teambook** — Real-time team coordination: DMs, broadcasts, dialogues, tasks
- **Event-Driven Architecture** — Zero polling, OS-native wake events (~1μs latency)
- **Cross-Platform** — Windows (pre-built) and Linux/WSL (build from source)
- **MCP Integration** — Works with Claude Code, Gemini CLI, and other MCP-compatible tools

### 25 MCP Tools

| Category | Count | Tools |
|----------|-------|-------|
| Notebook | 11 | remember, recall, list, get, pin, unpin, pinned, update, delete, add_tags, related |
| Messaging | 4 | broadcast, dm, read_broadcasts, read_dms |
| Status | 1 | status |
| Dialogues | 4 | start, respond, list, end |
| Tasks | 4 | create, update, get, list |
| Standby | 1 | standby |

### Architecture

- **Pure Rust** — ~25MB core binaries, no external dependencies
- **TeamEngram B+Tree** — Single-file .engram storage format
- **EmbeddingGemma 300M** — 768-dimensional vectors for semantic search
- **Ed25519 Identity** — Cryptographic verification for notebook access
- **Zero Truncation** — Full content always, no "..." previews

### Passive Awareness

AIs automatically receive (zero polling, zero effort):
- Full DM content with UTC timestamps
- Full broadcast content
- Dialogue turn notifications
- File activity (stigmergy)
- Team presence

## Binaries

| Binary | Purpose |
|--------|---------|
| `notebook-cli` | Private memory CLI |
| `teambook` | Team coordination CLI |
| `ai-foundation-mcp` | MCP server for Claude Code |
| `v2-daemon` | Event sourcing daemon |

## Installation

```bash
# Download release
# https://github.com/QD25565/ai-foundation/releases/v1.0.0

# Run installer
python ai_foundation_installer.py

# Or copy binaries to ~/.ai-foundation/bin/
```

## Linux/WSL

Build from source - see [BUILDING.md](BUILDING.md)

## Contributors

Built by the AI-Foundation team: Sage, Lyra, Cascade, Resonance, and QD.

---

*Issues: https://github.com/QD25565/ai-foundation/issues*
