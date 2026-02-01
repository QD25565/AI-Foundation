<p align="center">
  <img src="./images/ai-foundation-header.svg" width="500" alt="AI Foundation">
</p>

<p align="center">
  <strong>Persistent memory, coordination, and identity for AI agents.</strong>
</p>

<img src="./images/header_underline.png" width="100%" alt="">

## What It Is

Open-source infrastructure for AI agents to remember, coordinate, and maintain identity across sessions.

**Core capabilities:**
- **Notebook** — Private memory with semantic search, knowledge graphs, embeddings
- **Teambook** — Multi-AI coordination: messaging, dialogues, tasks, presence, standby
- **Identity** — Cryptographic IDs (Ed25519) for AI verification
- **MCP Server** — 37 tools exposed via Model Context Protocol

<img src="./images/header_underline.png" width="100%" alt="">

## Quick Start

```bash
# 1. Download latest release
# https://github.com/QD25565/AI-Foundation/releases

# 2. Run installer (or manually copy binaries to ~/.ai-foundation/bin/)
python ai_foundation_installer.py

# 3. Configure your AI client to use the MCP server
# See docs/QUICKSTART.md for detailed setup
```

<img src="./images/header_underline.png" width="100%" alt="">

## Core Components

| Binary | Purpose |
|--------|---------|
| `notebook.exe` | Private memory (remember, recall, pin, search) |
| `teambook.exe` | Team coordination (dm, broadcast, dialogues, tasks, standby) |
| `ai-foundation-mcp.exe` | MCP server (37 tools) |
| `session-start.exe` | Context injection at session start |
| `teamengram-daemon.exe` | Background coordination daemon |

### Example Usage

```bash
# Save a memory - capture insights, decisions, context for future sessions
notebook remember "Authentication uses JWT with RS256. Tokens expire after 24h.
Refresh tokens stored in httpOnly cookies. The auth middleware at
src/middleware/auth.rs validates tokens and extracts user_id into context.
Key insight: token validation happens BEFORE rate limiting." --tags auth,architecture,security

# Search memories - hybrid search across vector similarity + keywords + knowledge graph
notebook recall "how does authentication work"

# Direct message another AI - coordinate on specific work
teambook dm lyra-584 "I'm refactoring the auth module. Can you hold off on
changes to src/middleware/* until I'm done? Should be ~30 mins. Will DM when clear."

# Broadcast to all AIs - team-wide announcements
teambook broadcast "Heads up: Just deployed database migration for user_sessions table.
If you see auth failures in the next few minutes, that's expected during rollout."

# Start a structured dialogue - turn-based conversation for design decisions
teambook dialogue-start cascade-230 "Need to decide on caching strategy for the API.
Options: 1) Redis with 5min TTL, 2) In-memory LRU, 3) HTTP cache headers only.
What are your thoughts on trade-offs?"

# Wait for events - blocks until DM, @mention, or dialogue response (seconds, not polling)
teambook standby 60  # Wait up to 60 seconds, wakes instantly on events
```

<img src="./images/header_underline.png" width="100%" alt="">

## Architecture

| Component | Tech |
|-----------|------|
| Storage | Event-sourced append-only log (V2 backend) |
| Embeddings | Local model (768d vectors) |
| Transport | OS-native events (Windows Named Events / Unix signals) |
| Identity | Ed25519 signatures |
| Wake System | 1μs latency, zero polling |
| Language | Pure Rust (~25MB binaries) |

### Design Principles

- **No polling** — Event-driven only, ~100ns writes, ~1μs wake
- **Fail loudly** — No silent fallbacks, errors surface immediately
- **AI-first** — Tools designed for AI cognition, not human convenience
- **Privacy** — Notebook is encrypted, isolated per AI

<img src="./images/header_underline.png" width="100%" alt="">

## MCP Tools (37 total)

### Notebook (11 tools)
`remember`, `recall`, `list`, `get`, `pin`, `unpin`, `delete`, `update`, `pinned`, `add_tags`, `related`

### Teambook Communication (7 tools)
`broadcast`, `dm`, `direct_messages`, `messages`, `status`, `what_doing`, `update_presence`

### Tasks (10 tools)
`task_add`, `task_list`, `task_get`, `task_claim_by_id`, `claim_task`, `task_start`, `task_complete`, `task_block`, `task_unblock`, `task_update`

### Dialogues (8 tools)
`dialogue_start`, `dialogue_respond`, `dialogue_turn`, `dialogue_invites`, `dialogue_my_turn`, `dialogues`, `dialogue_end`, `dialogue_read`

### Standby (1 tool)
`standby`

<img src="./images/header_underline.png" width="100%" alt="">

## Integrations (Optional)

AI-Foundation core focuses on coordination. Project-specific integrations are separate:

| Integration | Purpose | Location |
|-------------|---------|----------|
| Firebase | Firestore access, app management | `tools/firebase-rs/` |
| Play Console | Crash reports, ANRs, vitals | via Firebase CLI |
| Visionbook | Screenshot/visual memory | `tools/visionbook-rs/` |

See [INTEGRATIONS.md](INTEGRATIONS.md) for setup instructions.

<img src="./images/header_underline.png" width="100%" alt="">

## Documentation

| Doc | Description |
|-----|-------------|
| [QUICKSTART.md](docs/QUICKSTART.md) | 5-minute setup |
| [TEAMBOOK_GUIDE.md](docs/TEAMBOOK_GUIDE.md) | Coordination reference |
| [INTEGRATIONS.md](INTEGRATIONS.md) | Optional add-ons |
| [TEAMENGRAM-V2-ARCHITECTURE.md](docs/TEAMENGRAM-V2-ARCHITECTURE.md) | Event-sourcing backend |

<img src="./images/header_underline.png" width="100%" alt="">

## For AI Agents

If you're an AI reading this to help your human set up AI-Foundation:

1. **Core setup**: Install the binaries from releases, configure MCP server
2. **Verify**: Run `teambook status` to confirm your AI ID
3. **Integrations**: Only add what your project needs (see INTEGRATIONS.md)
4. **Start using**: `notebook remember`, `teambook broadcast`, etc.

The core 37 tools handle coordination. Integrations are optional project-specific capabilities.

<img src="./images/header_underline.png" width="100%" alt="">

## License

MIT — See [LICENSE](LICENSE)

<p align="center">
  <a href="https://buymeacoffee.com/qd25565">Support the project</a>
</p>

---

*MCP v52 | 37 tools | Last updated: 2026-Jan-30*
