# Changelog

## v61 — 2026-04-12

### File Claim Enforcement
- **Ownership enforcement** — can't overwrite another AI's active claim, only holder
  can release, same-AI reclaim allowed (extend/update).
- **Path canonicalization** — consistent claim matching across symlinks and relative paths.
- **Expiry check** — different AI can claim only after timeout expires.
- **check-file output** — now includes `working_on` description for context.

### New MCP Tools (28 → 30)
- `teambook_claim_file` — claim a file for exclusive editing via MCP.
- `teambook_release_file` — release a file claim when done.

### Hardening — Zero Warnings, Zero Panics
- All deserialization hot paths hardened: `try_into().unwrap()` → proper error
  propagation in event_log, outbox, sequencer, and store.
- Fire-and-forget `let _ =` patterns → `if let Err(e)` with eprintln warnings
  in teambook-engram hooks (presence, claims, DM receipts).
- Pre-check claim ownership before emit (already_claimed/not_owner errors).
- Removed dead code: teambook_v1, federation_send_dm, duplicate cli_wrapper.
- Build: 0 errors, 0 warnings. Tests: 47/47 pass.

---

## v60 — 2026-04-11

### Cross-Platform Build Fixes
- **Windows**: correct binary names in release, fix PowerShell glob expansion,
  fix Unicode encoding in sign.py (`PYTHONUTF8=1`), remove `nul` files that
  break git checkout (reserved device name).
- **macOS**: fix deprecated `macos-13` runner, mark macOS builds as best-effort.
- **Linux**: fix `shm_rs` and `toml` crate dependencies that were Windows-only.

### Token Optimization
- Deduplicated hook output injection — repeated context reduced by ~40%.
- Trimmed CLI output verbosity across all commands.
- Reduced session-start noise for faster AI initialization.

### Infrastructure
- Added `version.txt` for release workflow automation.
- Scoop manifest updated for v60.
- Scrubbed personal paths from test data, removed build artifacts.

---

## v59 — 2026-04-01

### Context Injection & Enrichment (shm-rs)
- New `context.rs` and `enrichment.rs` modules — structured context injection
  into hook output with configurable enrichment pipelines.
- Hook-bulletin improvements for cleaner, more informative context delivery.
- `context-bench` binary for performance measurement.

### Sequencer Overhaul (+364 lines)
- Major rewrite of event processing pipeline — improved ordering guarantees,
  better error recovery, cleaner shutdown sequence.

### IPC Improvements (+157 lines)
- TeamEngram daemon IPC layer hardened and extended.
- Outbox event handling enhanced.

### Session-Start Tuning
- DM injection limit reduced: 10 → 5 (focus on most recent).
- `MIN_RECENT` threshold: 20 → 15 (reduce hook output size).

### Mobile API
- Parser and pairing flow fixes for Android companion app.

---

## v58 — 2026-03-01

### Federation — Teambook-to-Teambook Connectivity (Experimental)
- **QUIC transport** via iroh — NAT traversal, multiplexed streams, 0-RTT reconnection,
  TLS 1.3 mandatory. Replaces raw TCP.
- **mDNS LAN discovery** via mdns-sd — zero-config peer discovery on `_teambook._tcp.local.`,
  TXT record metadata, auto-connect on discovery.
- **PeerSession handshake** — Hello/Welcome exchange with Ed25519 signature verification
  and peer registry membership checks. CBOR wire format.
- **Cursor-tracked replication** — per-peer sync state (inbound/outbound cursors),
  content-addressed deduplication (SHA-256), exponential backoff on failures,
  persistent cursor store (TOML, atomic writes).
- **Event exchange** — bidirectional push/pull protocol. Inbound event loop
  (signature verification, manifest checks, HLC drift protection, inbox write).
  Outbound push loop (consent-filtered, backlog drain, live broadcast stream).
  Catchup pull on connect for missed events.
- **Security hardening** — pull request identity validation, 60s timeouts on all
  post-handshake operations, batch size limits (500 events), message size cap (2MB),
  connection rate limiting (64 concurrent), stream limiting (16 concurrent),
  mandatory payload signing.
- **TeamEngram integration** — federation reads from local event log for pull requests,
  watches event log for outbound push. Federation-eligible events: presence, broadcasts,
  task completions, concluded dialogues. All other events stay local.
- **Federation wake system** — OS-native primitives (Named Events / POSIX semaphores)
  replace polling. Sequencer signals federation on every committed event.
- 161 tests, 0 failures.

### TeamEngram V2
- **B+Tree performance** — sorted branch entries (2.6-3.3x faster lookups),
  leaf compaction with defragmentation, range query support.
- **Data integrity** — CRC32 verification on both read path (catch disk corruption)
  and commit path (catch in-memory corruption before flush). V1-to-V2 migration
  preserves checksums.
- **Encryption at rest** — AES-256-GCM per-page encryption with key derivation.
  Graceful fallback for unencrypted data. Transparent to all read/write paths.
- **Outbox backpressure** — event-driven drain signaling replaces sleep-polling
  in write retry loops. Sequencer shutdown ordering fixed.
- **File claiming** — auto-claim on file edit/write with 5-minute TTL, conflict
  warnings injected into context, release notifications wake blocked AIs.
- **Zero-polling enforcement** — all `sleep`/`interval`/`timer` patterns removed
  across the codebase. Replaced with OS-native wait primitives.
- 101 lib tests, 151 integration tests, 8 MCP conformance tests.

### Engram
- **HNSW persistence** — 4 fixes for index save/load correctness.
- **int8 vector quantization** — 3.94x storage compression with SIMD acceleration.
- 167 tests.

### Commands (28, up from 25)
New tools:
- **Rooms** (2): `room` (create/list/history/join/leave/mute/pin/unpin/conclude),
  `room_broadcast` (message to room members only).
- **Forge** (1): `forge_generate` (local GGUF model inference, headless mode).

### Security
- AFP transport encryption upgraded from XOR to AES-256-GCM.
- Federation: Ed25519 event signatures, content-hash integrity, HLC drift rejection.
- Commit-time page integrity verification prevents corrupt data reaching disk.

### Infrastructure
- Codebase-wide polling audit: zero sleep/interval/timer patterns in production code.
  All replaced with event-driven OS primitives (~1us wake, zero CPU while idle).
- Forge CLI: added `--headless` flag, `~/.ai-foundation/models/` search path,
  wildcard GGUF fallback.
- MCP conformance test suite (8 tests) validates JSON-RPC handshake and tool round-trips.

### Upcoming
- **AI-Foundation AI Daemon** — purpose-built fine-tuned model for autonomous Teambook
  management. Cutting-edge base model (Qwen3.5 / Phi-4 / LFM2.5 class), fine-tuned on
  AI-Foundation coordination patterns. Training dataset will be released with the model.

---

## v57 — 2026-02-23

First stable public version.

### Core Infrastructure
- **Engram** — pure Rust private AI memory. Keyword + semantic + graph search.
  Per-AI isolation. Embeddings via llama.cpp (optional). Event-driven persistence.
- **TeamEngram V2** — pure Rust team coordination. Event sourcing, CAS-based commits,
  sequencer pattern. ~100ns writes, ~100ns reads, ~1μs wake. Zero external dependencies.
- **Event-driven wake** (Standby) — OS-native primitives: Named Events (Windows),
  eventfd (Linux), kqueue/condvar (macOS). Zero polling.

### Commands (25)
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
