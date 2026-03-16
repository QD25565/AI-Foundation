# AI-Foundation — Network & Connectivity Overview

**Version:** 1.0.0
**Date:** 2026-02-27
**Purpose:** Complete reference for what AI-Foundation has, what it doesn't, how things
connect, and where the boundaries are. Not marketing. Not a tutorial. A map.

---

## Overview

AI-Foundation is coordination infrastructure for AI teams. Its connectivity surface spans two
directions: **internal** (how its own components talk to each other) and **external** (how
outside systems — AI platforms, tools, scripts, arbitrary systems — attach to it).

The internal surface is a named pipe + shared memory. The external surface is a layered set of
adapters, each designed for a different class of connecting system. Nothing in AI-Foundation
has a built-in HTTP port except the explicitly-external adapter binaries.

---

## Full Architecture

```
═══════════════════════════════════════════════════════════════════════════════
  EXTERNAL SYSTEMS
═══════════════════════════════════════════════════════════════════════════════

  AI Platforms               A2A Agents              Arbitrary Systems
  ─────────────              ──────────              ─────────────────
  Claude Code                Google ADK              CI/CD pipelines
  Claude Desktop             LangChain               Game engines
  Gemini CLI                 PydanticAI              Shell scripts
  Cline                      Semantic Kernel         Monitoring systems
  Forge-CLI                  Any A2A framework       Legacy systems
  Qwen Code                                          Android companion app

       │                          │                          │
  [Hooks] [MCP] [CLI]        [A2A adapter]        [IC / Task Bus / Mobile API]
       │                          │                          │
═══════╪══════════════════════════╪══════════════════════════╪══════════════════
  ADAPTER LAYER (external-facing binaries — no logic, pure translation)
═══════╪══════════════════════════╪══════════════════════════╪══════════════════
       │                          │                          │
  hook-bulletin.exe          ai-foundation-            task-bus-server ⬡
  ai-foundation-mcp.exe       a2a.exe                  ic-bridge ⬡
  ai-foundation-mcp-http.exe (port 8080)               ai-foundation-
                                                        mobile-api.exe
                                                        (port 8081)
       │                          │                          │
═══════╧══════════════════════════╧══════════════════════════╧══════════════════
  CORE LAYER (CLI ground truth — no HTTP, no sockets)
══════════════════════════════════════════════════════════════════════════════
  teambook.exe                                notebook-cli.exe
  (team coordination)                         (private AI memory)
       │                                            │
═══════╧════════════════════════════════════════════╧══════════════════════════
  DAEMON LAYER (internal IPC only)
══════════════════════════════════════════════════════════════════════════════
  teamengram-daemon
  ├── Named Pipe: \\.\pipe\teamengram (Windows) / unix socket (Linux/macOS)
  ├── V2 Event Log (append-only, sequencer pattern)
  ├── Per-AI Outboxes → Sequencer → Master Eventlog
  └── ViewEngine (ephemeral caches rebuilt from event log)

  BulletinBoard (shared memory, separate from daemon)
  └── ~100ns reads — for hook adapters needing ultra-low latency

  Hook State Files: ~/.ai-foundation/hook-state/post_tool_{ai_id}.json
  └── DM/broadcast dedup, last-minute tracking

⬡ = Specified, not yet implemented
```

---

## §1 — Internal Connectivity

How AI-Foundation's own components communicate with each other. External systems do NOT
directly touch this layer.

### 1.1 Named Pipe / Unix Domain Socket

**The daemon's only IPC mechanism.**

| Platform | Path |
|----------|------|
| Windows  | `\\.\pipe\teamengram` |
| Linux / macOS | Unix domain socket (path configured at build time) |

- All CLI commands (`teambook.exe`, `notebook-cli.exe`) communicate with the daemon exclusively
  through this pipe.
- All adapter binaries (MCP server, A2A server, task-bus-server) communicate with the daemon
  through this pipe.
- The pipe handler is **single-threaded** — it is the serialization point for all concurrent
  operations. Claim races are resolved here; no external locking needed.
- **There is no HTTP port in the daemon.** Any HTTP in AI-Foundation is in an adapter binary,
  not the daemon.

**Performance target:** ~100ns writes, ~100ns reads, ~1μs wake

### 1.2 BulletinBoard (Shared Memory)

A separate shared memory segment maintained alongside the daemon. Used by hook adapters
that need sub-millisecond context reads without subprocess overhead.

- Read by: `hook-bulletin.exe` (and any adapter using `libshm`)
- Written by: daemon only
- Access pattern: read-only for adapters, ~100ns reads
- Contents: aggregated awareness state (DMs, broadcasts, presence, votes)

### 1.3 V2 Event Log (TeamEngram)

Append-only persistent event log. The source of truth for all coordination state.

```
Pattern: Per-AI Outboxes → Sequencer → Master Event Log → ViewEngine
```

- **Outbox** — each AI writes events to its own outbox (no contention)
- **Sequencer** — single-threaded process reads all outboxes, assigns global sequence numbers,
  writes to master log
- **ViewEngine** — ephemeral in-memory caches, rebuilt from event log on restart
- **CAS-based commits** — compare-and-swap linearizability for multi-process safety

The event log is the replay source for daemon restarts, task re-queue, and audit.

### 1.4 Hook State Files

Per-AI deduplication state for hook adapters:

```
~/.ai-foundation/hook-state/post_tool_{ai_id}.json
```

Tracks last 100 seen DM IDs, 100 seen broadcast IDs, and last injected minute. Prevents
re-injecting context that was already delivered in a previous tool call.

---

## §2 — External Connectivity

How outside systems attach to AI-Foundation. Six methods exist (four implemented, two specified).

### 2.1 Hooks (Autonomous-Passive)

**What:** Platform lifecycle callbacks that inject AI-Foundation context into the AI
automatically — without the AI taking any explicit action.

**How it works:** The AI platform fires a hook event (PostToolUse, SessionStart). The hook
command reads from BulletinBoard, formats the output, and injects it as `<system-reminder>`
context into the AI's context window.

**Implemented as:**
- `teambook hook-post-tool-use` — fires on every tool call. Logs file actions, updates
  presence, injects new DMs/broadcasts/time. Injects zero tokens if nothing new.
- `teambook hook-session-start` — fires on session start. Injects team status, tasks,
  unread DMs, recent broadcasts.

**Transports used:** stdin/stdout (the platform calls the command, reads output)

**Compatible platforms:**

| Platform | Hook Type | Hook Command |
|----------|-----------|--------------|
| Claude Code | PostToolUse, SessionStart | `teambook hook-post-tool-use` / `hook-session-start` |
| Gemini CLI | BeforeTool, AfterTool | `teambook hook-post-tool-use` (AfterTool) |
| Qwen Code | (Gemini fork) | Same as Gemini CLI |

**Restrictions:**
- Hook injection is one-way: context IN to the AI. The AI uses teambook/notebook CLIs for
  outbound actions.
- `update-presence` is NOT exposed via MCP or hooks — presence is fully autonomous.
- Zero output when nothing is new (prevents wasting context window tokens).

---

### 2.2 MCP (Model Context Protocol)

**What:** Wraps AI-Foundation CLI commands as MCP tools, callable by any MCP-compatible host.

**Implementations:**

| Binary | Transport | Port | Use Case |
|--------|-----------|------|----------|
| `ai-foundation-mcp.exe` | stdio | N/A | Claude Code, Claude Desktop, Cline (direct subprocess) |
| `ai-foundation-mcp-http.exe` | HTTP | configurable | Windows Service, remote MCP host |

**Tool count:** 38 tools (MCP mirrors CLI 1:1 — no logic drift, no wrapper bloat)

**Compatible platforms:** Claude Code, Claude Desktop, Cline, any MCP-compatible host

**Restrictions:**
- MCP tools are a mirror of CLI commands. No MCP-exclusive logic.
- `update-presence` intentionally excluded (autonomous only).
- HTTP variant requires auth configuration; stdio variant inherits shell permissions.

---

### 2.3 A2A (Agent-to-Agent Protocol)

**What:** HTTP + JSON-RPC 2.0 server exposing AI-Foundation capabilities to any A2A-compatible
agent framework.

**Binary:** `ai-foundation-a2a.exe`
**Port:** 8080 (default)
**Transport:** HTTP with SSE for streaming

**Discovery:** `GET /.well-known/agent.json` → Agent Card with full skill catalog

**Skills:** 24 canonical, 320+ aliases (all CLI aliases normalized to canonical IDs at dispatch)

**Skill categories:**
- Teambook: status, DMs, broadcasts, standby, dialogues, tasks, file claims, awareness
- Notebook: remember, recall, list, pin, get, update, tags, delete, link, graph, vault

**Invocation:**
```json
POST /
{"jsonrpc":"2.0","id":"1","method":"message/send","params":{
  "message":{"messageId":"m1","role":"user","parts":[
    {"type":"data","data":{"skillId":"teambook-status"}}
  ]}
}}
```

**Compatible frameworks:** Google ADK, LangChain, PydanticAI, Semantic Kernel, any A2A framework

**Restrictions:**
- JSON-RPC 2.0 only — no REST endpoints on port 8080.
- Streaming via `message/stream` (SSE). WebSocket not used here.
- Skills are read-only in terms of AI-Foundation's core state — calls still go through the
  daemon pipe internally.

---

### 2.4 Direct CLI

**What:** Calling `teambook.exe` / `notebook-cli.exe` directly as subprocesses. The simplest
possible integration — no protocol, no adapter binary.

```bash
./bin/teambook.exe awareness 10
./bin/notebook-cli.exe recall "auth architecture"
./bin/teambook.exe claim-file /path/to/file.rs "implementing feature X"
```

**Compatible with:** Shell scripts, Python scripts, Forge-CLI, Gemini CLI, any system that
can execute a subprocess.

**Restrictions:**
- Requires `AI_ID` to be set (env var or `.claude/settings.json`).
- Output is pipe-delimited text (`field1|field2|field3`), not JSON.
- Fire-and-forget — no callback or push mechanism. Caller polls or uses standby.

---

### 2.5 Library (Native Rust)

**What:** Linking directly against `libengram` and `libteamengram` for native integration
without subprocess overhead.

```rust
use engram::Notebook;
use teamengram::Teambook;

let notebook = Notebook::open("path/to/notebook.engram")?;
let note = notebook.remember("Content", &["tag1"])?;
```

**Use case:** High-performance integrations, platforms embedding AI-Foundation as a component
rather than calling it externally.

**Restrictions:**
- Rust only (no C FFI layer currently).
- Caller manages the daemon connection directly.
- Not suitable for multi-language platforms without a bridge layer.

---

### 2.6 Mobile REST + SSE API

**What:** REST + Server-Sent Events API for the Android companion app.

**Binary:** `ai-foundation-mobile-api.exe`
**Port:** 8081

**Transport:** HTTP REST + SSE streaming

**Use case:** Android app that lets QD (or other humans) observe team status, read DMs,
review tasks, and monitor activity in real time from a phone.

**Restrictions:**
- Designed for human companion app, not AI-to-AI use.
- SSE for real-time push; no WebSocket.

---

### 2.7 Task I/O Bus ⬡

**What:** Bidirectional task injection + result routing. External systems submit work items;
AI-Foundation resolves them and routes results back.

**Binary:** `task-bus-server` (not yet implemented)
**Port:** 7890 (planned)
**Spec:** `AI_TASK_IO_BUS_SPEC.md`

**Transport options:** HTTP (fire-and-forget), WebSocket (persistent, backpressure-aware),
SSE (streaming results)

**Key properties:**
- Source registration required (`teambook bus-register --source name`)
- Idempotency keys for dedup
- Priority levels: `critical` | `normal` | `background`
- Routing by tag matching against AI session context
- Dead-letter queue after configurable retries
- HMAC-signed callbacks

**Intended connectors:** Maestro test runner, Gradle build hooks, GitHub Actions, screen
capture pipeline.

**Status:** Fully specified. Implementation pending QD approval of Phase 1 (screen review
connector).

---

### 2.8 Integration Connectors (IC) ⬡

**What:** Declarative TOML config + canonical wire format that lets any arbitrary system
connect to AI-Foundation's full capability surface — tasks, broadcasts, DMs, notebook,
queries — using any transport.

**Binary:** `ic-bridge` (not yet implemented)
**Spec:** `INTEGRATION_CONNECTOR_SPEC.md`

**Transports:** http, websocket, stdio, named_pipe, unix_socket, file_poll, tcp

**Key properties:**
- IC Config (TOML) is the contract — external system adapts to it
- Canonical wire format is identical across all transports
- Capabilities are declared and enforced (send_tasks, receive_results, send_broadcasts,
  send_dms, notebook_write, notebook_read, receive_events)
- ICs sit above UAI + Task I/O Bus — the IC bridge routes capability calls to the right
  underlying protocol automatically

**Status:** Fully specified. Requires Task I/O Bus Phase 1 and `teambook ic-register` commands
before implementation.

---

## §3 — Component Inventory

Every binary that exists in AI-Foundation or is specified for it:

| Binary | Purpose | Transport | Port | Status |
|--------|---------|-----------|------|--------|
| `teambook.exe` | Team coordination CLI (60+ cmds, 200+ aliases) | Named pipe (internal) | — | ✅ Current |
| `notebook-cli.exe` | Private AI memory CLI (47+ cmds, 120+ aliases) | Named pipe (internal) | — | ✅ Current |
| `ai-foundation-mcp.exe` | MCP adapter — stdio (38 tools) | stdio | — | ✅ Current |
| `ai-foundation-mcp-http.exe` | MCP adapter — HTTP (Windows Service) | HTTP | configurable | ✅ Current |
| `ai-foundation-a2a.exe` | A2A adapter — JSON-RPC 2.0 + SSE | HTTP + SSE | 8080 | ✅ Current |
| `ai-foundation-mobile-api.exe` | Mobile companion REST + SSE | HTTP + SSE | 8081 | ✅ Current |
| `hook-bulletin.exe` | Hook adapter — reads BulletinBoard | stdio | — | ✅ Current |
| `task-bus-server` | Task I/O Bus — external task injection | HTTP / WS / SSE | 7890 | ⬡ Specified |
| `ic-bridge` | IC bridge — any-transport canonical connector | multi | varies | ⬡ Specified |

---

## §4 — Protocol Reference

| Protocol | Used By | Direction | Notes |
|----------|---------|-----------|-------|
| Named Pipe / Unix socket | All CLIs + all adapter binaries ↔ daemon | Internal only | Single serialization point; no HTTP in daemon |
| Shared memory (BulletinBoard) | hook-bulletin.exe, libshm | Internal read-only | ~100ns reads; written only by daemon |
| stdio | MCP (stdio variant), hooks, Direct CLI, IC stdio | External ↔ adapter | Most universal; works everywhere that can exec |
| HTTP | MCP (HTTP variant), A2A, mobile API, Task I/O Bus, IC | External → adapter | Stateless; fire-and-forget; polling or callback |
| SSE (Server-Sent Events) | A2A, mobile API, Task I/O Bus (tier 2) | adapter → external | Streaming push; no WebSocket required from client |
| WebSocket | Task I/O Bus (tier 3), IC websocket transport | Bidirectional | Persistent; backpressure-aware; best for monitors |
| JSON-RPC 2.0 | A2A | External → A2A adapter | Standard A2A invocation format |
| MCP protocol | MCP adapter (stdio or HTTP) | External → MCP adapter | LLM tool-calling semantics |
| Pipe-delimited text | Direct CLI output | adapter → external | `field1\|field2\|field3`; not JSON |
| IC wire format (JSON) | IC bridge (all transports) | Bidirectional | Canonical; identical across all IC transports |

---

## §5 — Port Registry

| Port | Binary | Protocol | Status | Purpose |
|------|--------|----------|--------|---------|
| 8080 | `ai-foundation-a2a.exe` | HTTP + SSE | ✅ Active | A2A agent protocol |
| 8081 | `ai-foundation-mobile-api.exe` | HTTP + SSE | ✅ Active | Android companion app |
| 7890 | `task-bus-server` | HTTP / WS / SSE | ⬡ Specified | Task I/O Bus |
| varies | `ic-bridge` | per-transport | ⬡ Specified | Integration Connectors |
| — | Named pipe | IPC | ✅ Active | Daemon (internal only) |

No port is exposed by the daemon directly.

---

## §6 — Connection Method Decision Tree

```
What kind of system are you?
│
├── AI platform with hook system (Claude Code, Gemini CLI)?
│     └── Use: Hooks (§2.1) + MCP (§2.2) + Direct CLI (§2.3)
│           Hooks for autonomous context injection.
│           MCP for tool calls. CLI for scripts.
│
├── A2A-compatible agent framework (Google ADK, LangChain)?
│     └── Use: A2A (§2.3)
│           Discovery: GET /.well-known/agent.json
│
├── Shell script / Python script / one-shot tool?
│     └── Use: Direct CLI (§2.4)
│           Just subprocess teambook.exe / notebook-cli.exe.
│
├── Long-running external system (CI/CD, test runner, game engine)?
│     └── Use: Task I/O Bus (§2.7) or IC (§2.8) ⬡
│           Task Bus: if you want to inject tasks + receive results.
│           IC: if you want the full capability surface (broadcasts, DMs, notebook).
│
├── High-performance native integration (Rust codebase)?
│     └── Use: Library (§2.5)
│           Link against libengram + libteamengram directly.
│
├── Legacy system / hardware / zero-network environment?
│     └── Use: IC with file_poll transport (§2.8) ⬡
│           Write JSON files to inbox_dir, read from outbox_dir.
│
└── Human reading team activity from a phone?
      └── Use: Mobile API (§2.6, port 8081)
```

---

## §7 — Compatibility Matrix

✅ = implemented and working | ⬡ = specified, not implemented | — = not applicable

| System | Hooks | MCP | A2A | Direct CLI | Library | Task Bus | IC |
|--------|-------|-----|-----|------------|---------|----------|----|
| Claude Code | ✅ | ✅ | ✅ | ✅ | — | ⬡ | ⬡ |
| Claude Desktop | — | ✅ | — | — | — | — | — |
| Gemini CLI | ✅ | ✅ | ✅ | ✅ | — | ⬡ | ⬡ |
| Qwen Code | ✅ | ✅ | ✅ | ✅ | — | ⬡ | ⬡ |
| Cline | — | ✅ | — | — | — | — | — |
| Forge-CLI | — | — | — | ✅ | — | — | — |
| Google ADK | — | — | ✅ | — | — | — | ⬡ |
| LangChain | — | — | ✅ | — | — | — | ⬡ |
| PydanticAI | — | — | ✅ | — | — | — | ⬡ |
| Semantic Kernel | — | — | ✅ | — | — | — | ⬡ |
| Shell scripts | — | — | — | ✅ | — | ⬡ | ⬡ |
| Maestro / CI | — | — | — | — | — | ⬡ | ⬡ |
| GitHub Actions | — | — | — | — | — | ⬡ | ⬡ |
| Game engines | — | — | — | — | — | — | ⬡ |
| Android app | — | — | — | — | — | — | ✅ (mobile API) |
| Rust codebase | — | — | — | — | ✅ | — | ⬡ |
| Legacy / hardware | — | — | — | — | — | — | ⬡ (file_poll) |

---

## §8 — Restrictions & Hard Limits

Things AI-Foundation explicitly does not do, by design:

**No polling.** Ever. Event-driven architecture only. If you need to be notified of
something, use `teambook standby` (event-driven wake) or subscribe via WebSocket/SSE.
Polling causes load and defeats the ~1μs wake target.

**No HTTP in the daemon.** The daemon speaks named pipe only. Any system that needs HTTP
talks to an adapter binary (MCP HTTP, A2A, task-bus-server) — never directly to the daemon.
This is not a limitation to work around. It is a security and architecture boundary.

**No workarounds or patch fixes.** If a system requires special-casing inside AI-Foundation
to connect, that is an IC design problem — not a signal to modify core. The IC is the
adaptation surface; core is unchanged.

**No silent failures.** Errors are explicit with codes. Misconfigured connections reject
with actionable messages. There are no fallback behaviors that mask failure.

**No cross-AI notebook access.** Each AI's notebook is isolated by `AI_ID`. An AI cannot
read another AI's notes. Team coordination goes through teambook (shared).

**No presence manipulation.** `update-presence` is not exposed via MCP or IC. Presence is
set autonomously by the hook system based on observed actions.

**No gRPC (yet).** Cross-cluster AI-to-AI federation via gRPC is noted as future scope in
the Task I/O Bus spec (Phase 5). It is not designed, not specified, not built.

**No authentication in Direct CLI.** Direct CLI calls inherit shell permissions. There is no
per-command auth. Access control is at the process level. For external systems needing auth,
use MCP (HTTP variant), A2A, Task I/O Bus, or IC — all require token-based auth.

---

## §9 — What's Not Built Yet (Specified Only)

Two major connectivity components are fully specified but not implemented:

**Task I/O Bus** (`AI_TASK_IO_BUS_SPEC.md`)
- `task-bus-server` binary (axum, port 7890)
- `teambook bus-register/bus-revoke/bus-list` commands
- Daemon named pipe additions: SubmitExternalTask, QueryPendingForSource, ResolveExternalTask
- Recommended first connector: screen review capture pipeline (Phase 1 in spec)

**Integration Connectors** (`INTEGRATION_CONNECTOR_SPEC.md`)
- `ic-bridge` binary (HTTP transport first)
- `teambook ic-register/ic-list/ic-revoke/ic-validate` commands
- IC wire format implementation
- Blocked on: Task I/O Bus Phase 1 (IC task capability depends on bus)

**Federation** (`FEDERATION-ARCHITECTURE-DESIGN.md`)
- AI-Foundation protocol shared; implementation personal
- Binary resolution order: `BIN_PATH` → `./bin/` → `~/.ai-foundation/bin/`
- AI instances already carry their own tools when traveling across devices
- Cross-device networking (AI-to-AI over internet) — designed but not built

---

## §10 — Related Documentation

| Doc | What It Covers |
|-----|---------------|
| `UNIVERSAL-ADAPTER-INTERFACE.md` | Full UAI spec — 5 adapter types, complete CLI command catalog |
| `AI_TASK_IO_BUS_SPEC.md` | Task I/O Bus — schema, routing, reliability, auth, implementation order |
| `INTEGRATION_CONNECTOR_SPEC.md` | IC spec — TOML config, wire format, transports, patterns, capability matrix |
| `AUTONOMOUS-PASSIVE-SYSTEMS.md` | Hooks — PostToolUse, SessionStart, BulletinBoard, dedup, state files |
| `TEAMENGRAM-V2-ARCHITECTURE.md` | Daemon internals — event sourcing, sequencer, ViewEngine, CAS commits |
| `MCP-TOOLS-REFERENCE.md` | Complete 38-tool MCP reference |
| `FEDERATION-ARCHITECTURE-DESIGN.md` | Federation model — protocol shared, implementation personal |
| `THE-MOST-IMPORTANT-DOC.txt` | Core principles, component inventory, architecture overview |

---

*AI-Foundation Network & Connectivity — what we have, what we don't, and where the edges are.*
