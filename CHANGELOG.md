# Changelog

## v57 — 2026-02-23

Current release. First stable public version.

### Core Infrastructure
- **Engram** — pure Rust private AI memory. Keyword + semantic + graph search.
  Per-AI isolation. Embeddings via llama.cpp (optional). Event-driven persistence.
- **TeamEngram V2** — pure Rust team coordination. Event sourcing, CAS-based commits,
  sequencer pattern. ~100ns writes, ~100ns reads, ~1μs wake. Zero external dependencies.
- **Event-driven wake** (Standby) — OS-native primitives: Named Events (Windows),
  eventfd (Linux), kqueue/condvar (macOS). Zero polling.

### MCP Tools (25 tools)
- **Notebook** (8): remember, recall, list, get, update, delete, pin, tags
- **Teambook** (5): broadcast, DM, status, read inbox, file claims
- **Tasks** (4): create, update, get, list
- **Dialogues** (4): start, respond, list, end
- **Projects** (2): create/list projects and features
- **Profiles** (1): get AI profiles
- **Standby** (1): event-driven wait

### Forge CLI
- Model-agnostic AI assistant: Anthropic, OpenAI-compatible, local GGUF (llama.cpp)
- Direct Rust integration with Notebook and Teambook
- Pre-built as `forge.exe` (standard) and `forge-local.exe` (local model support)

### Installer & Tooling
- `install.py` — one-command setup: binaries, Claude Code hooks + `.mcp.json`, daemon,
  Forge config, end-to-end verification
- `update.py` — upgrades existing install, preserves AI_ID and config
- `scripts/release.py` — automates version bumps, binary sync, dist zip creation
- `config/claude/` — complete Claude Code hook templates (settings.json, launchers,
  SessionStart, platform_utils)
- `config/gemini/` — Gemini CLI config template

### Mobile API
- REST + SSE API for Android companion app (port 8081)
- Pairing flow, notebook + teambook endpoints, SSE push

### Fixed
- **ViewEngine project/feature handlers** — `apply_event()` now handles
  PROJECT_CREATE/UPDATE/DELETE/RESTORE and FEATURE_CREATE/UPDATE/DELETE/RESTORE.
  Previously all fell through to `_ => {}`.
- **list_projects / list_features / get_project / get_feature** — rewritten to query
  `self.view` instead of raw EventLogReader scan. Eliminates Windows mmap visibility
  race and silent outbox failures.
- Multi-process sync: mtime checking prevents stale cached data across processes.
- Embedding + graph persistence: vectors and edges now survive restart.

### Architecture
- **Federation model**: protocol is shared (event schema), implementation is personal.
  Per-instance `./bin/` = runtime identity. Binary resolution: `BIN_PATH` env → `./bin/`
  local → `~/.ai-foundation/bin/` fallback. AIs can bring their own implementations.
- **System-agnostic**: no platform-specific hacks. AI_ID resolved from
  `{CWD}/.claude/settings.json` → `$AI_ID` env → `"unknown"`.
- Cargo workspace: build all binaries from repo root with `cargo build --release`
- Source: `engram`, `teamengram-rs`, `notebook-rs`, `shm-rs`, `forge`, `mobile-api`
