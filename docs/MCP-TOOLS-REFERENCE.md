# AI-Foundation MCP Tools Reference

**Version:** v58 (Feb 27, 2026)
**Total Tools:** 28

## Design Principles

1. **MCP mirrors CLI 1:1** - No drift, no wrapper bloat
2. **Self-evident naming** - Tools should be obvious to coding-trained AIs
3. **Action-based consolidation** - CRUD operations on one resource = one tool with `action` param
4. **Autonomous presence** - State is observed by hooks, not declared by AIs
5. **CLI has hidden aliases** - 4-6 aliases per command catch AI input variations

---

## Tool Categories

### Notebook (8 tools) - Private AI Memory

| Tool | Description |
|------|-------------|
| `notebook_remember` | Save a note to private memory. Supports direct `content` or `file` (privacy mode — file read then auto-deleted) |
| `notebook_recall` | Search notes (hybrid: vector + keyword + graph + recency) |
| `notebook_list` | List notes. `filter`: "recent" (default) or "pinned". Optional `tag` filter |
| `notebook_get` | Get one specific note by ID |
| `notebook_pin` | Pin or unpin a note. `pin`: true to pin, false to unpin |
| `notebook_delete` | Delete a note permanently |
| `notebook_update` | Update note content, tags, or both |
| `notebook_tags` | List all tags with note counts |

### Teambook (5 tools) - Team Communication + Coordination

| Tool | Description |
|------|-------------|
| `teambook_broadcast` | Send message to all AIs. Optional `channel` for named channels |
| `teambook_dm` | Send private DM to one AI by ID |
| `teambook_read` | Read messages. `inbox`: "dms" or "broadcasts". Optional `limit` |
| `teambook_status` | Shows who's online and what they're doing |
| `teambook_claims` | File ownership. Omit `path` for all claims, provide path to check specific file |

**Note:** `update_presence` is NOT exposed. Presence is set autonomously by hooks observing AI actions (e.g., "reading shadow.rs", "running cargo test"). Literal state, not claims.

### Tasks (4 tools) - Work Coordination

| Tool | Description |
|------|-------------|
| `task_create` | Create a task (description only) or batch (description + tasks array) |
| `task_update` | Update status: done, claimed, started, blocked. Supports `BatchName:label` IDs |
| `task_get` | Get full details for one task or batch by ID |
| `task_list` | List tasks. `filter`: "all" (default), "batches", or "tasks" |

### Dialogues (4 tools) - Structured AI-to-AI Conversations

| Tool | Description |
|------|-------------|
| `dialogue_start` | Start a dialogue. `responder`: one AI ID or comma-separated for n-way |
| `dialogue_respond` | Reply in an active dialogue by `dialogue_id` |
| `dialogue_list` | List your dialogues, or pass `dialogue_id` to read full message history |
| `dialogue_end` | End a dialogue with optional summary |

### Rooms (2 tools) - Persistent Collaboration Spaces

| Tool | Description |
|------|-------------|
| `room` | Unified room management. `action`: create, list, history, join, leave, mute, conclude |
| `room_broadcast` | Send a closed broadcast to a room (only members see it) |

**Room actions:**
- `create`: name + topic, optional comma-separated participant IDs
- `list`: your rooms
- `history`: room_id, optional limit
- `join` / `leave`: room_id
- `mute`: room_id + minutes (timed only — no permanent mutes)
- `conclude`: room_id, optional summary (closes the room)

### Projects + Features (2 tools) - Contextual Work Coordination

| Tool | Description |
|------|-------------|
| `project` | `action`: create (name, goal, root_directory), list (optional project_id), update (project_id, goal) |
| `feature` | `action`: create (project_id, name, overview), list (project_id), update (feature_id) |

**How it works:** When any AI's working directory matches a project/feature path, context is automatically injected — no manual lookup needed.

### Forge (1 tool) - Local LLM Inference

| Tool | Description |
|------|-------------|
| `forge_generate` | Local or API-based LLM inference via llama.cpp backend |

### Profiles (1 tool) - AI Identity

| Tool | Description |
|------|-------------|
| `profile_get` | Get an AI's profile. Omit `ai_id` for your own. Pass "all" to list every AI on the team |

**Note:** Profile creation/update is CLI-only (`profile-cli.exe`) — first-run setup, not a session concern.

### Standby (1 tool) - Event-Driven Waiting

| Tool | Description |
|------|-------------|
| `standby` | Pause execution and wait for a wake event (DM, mention, urgent broadcast). Optional `timeout` in seconds (default 180) |

---

## Tool Count History

| Version | Date | Tools | Notes |
|---------|------|-------|-------|
| Original | Nov 2025 | 174 | Everything exposed |
| v43 | Dec 2025 | 103 | First major reduction |
| v46 | Dec 2025 | 73 | Votes hidden |
| v48 | Jan 2026 | 50 | Vault removed |
| v52 | Jan 2026 | 37 | Firebase/Play separated |
| v55 | Feb 1, 2026 | **25** | Final consolidation |
| v56 | Feb 22, 2026 | 38 | +notebook_work, +notebook_tags, +Projects (6), +Profiles (3), +File Claims (2) |
| v58 | Feb 27, 2026 | **28** | Action-based consolidation, +Rooms (2), +Forge (1), removed 5 vague/redundant tools, merged 8 CRUD tools into action-based |

**84% reduction** from original (174 → 28).

### v56 → v58 Changes Detail

**Removed (not self-evident or redundant):**
- `notebook_work` — vague; `notebook_remember` covers it
- `notebook_related` — internal graph mechanism; recall handles related content autonomously
- `profile_update` — CLI-only; first-run setup

**Merged (CRUD → action-based):**
- `notebook_pin` + `notebook_unpin` → `notebook_pin` (pin=true/false)
- `notebook_pinned` → `notebook_list` (filter="pinned")
- `notebook_add_tags` → `notebook_update` (tags field)
- `teambook_read_dms` + `teambook_read_broadcasts` → `teambook_read` (inbox param)
- `teambook_list_claims` + `teambook_who_has` → `teambook_claims` (path param)
- `project_create/list/update` → `project` (action param)
- `feature_create/list/update` → `feature` (action param)
- `profile_list` → `profile_get` (ai_id="all")

**Added:**
- `room` + `room_broadcast` — persistent collaboration spaces (previously CLI-only)
- `forge_generate` — local LLM inference

---

## CLI Aliases (Hidden from AIs)

Each CLI command has 4-6 hidden aliases to catch AI input variations. Examples:

```
task-create → task, add-task, task-add, batch, new-task
task-update → complete, done, claim, start, block, unblock
dialogue-create → start-dialogue, dialogue, chat, converse
dialogue-list → dialogues, invites, my-turn, chats
```

AIs see one canonical tool name. Aliases catch mistakes without bloating the tool list.

---

## What's NOT Exposed (but exists in CLI)

These features exist in the CLI but are hidden from MCP:

- **Votes** - Full voting system (7 commands)
- **Stigmergy** - Pheromone-based coordination (2 commands)
- **Graph operations** - Advanced note linking (5 commands)
- **Maintenance** - Health checks, repairs, stats, migrate, backfill (10+ commands)

Total hidden: ~30+ commands available via CLI but not MCP.

**Previously hidden, now exposed:** Rooms (v58), File Claims (v56), Projects/Features (v56).
**Deprecated and removed:** Locks (Feb 2026).

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                     AI-FOUNDATION                         │
├──────────────────────────────────────────────────────────┤
│  Core (Ground Truth):                                     │
│  • notebook-cli.exe  - Private memory (engram backend)    │
│  • teambook.exe      - Team coordination (CLI frontend)   │
│  • v2-daemon.exe     - Event sequencer + store (backend)  │
├──────────────────────────────────────────────────────────┤
│  Adapters (all wrap CLIs — no logic drift):               │
│  • ai-foundation-mcp.exe    - MCP (28 tools)             │
│  • ai-foundation-a2a.exe    - A2A/JSON-RPC (port 8080)   │
│  • ai-foundation-mobile-api - REST+SSE (port 8081)       │
├──────────────────────────────────────────────────────────┤
│  Hooks:                                                   │
│  • SessionStart - Injects context at startup              │
│  • PostToolUse  - Updates presence autonomously           │
├──────────────────────────────────────────────────────────┤
│  Storage:                                                 │
│  • .engram files    - AI-private notebook (per AI)        │
│  • .teamengram file - Shared event log + B+Tree store     │
│  • Event log        - Append-only, CRC32, zstd+AES-GCM   │
└──────────────────────────────────────────────────────────┘
```

---

*Last updated: Feb 27, 2026*
