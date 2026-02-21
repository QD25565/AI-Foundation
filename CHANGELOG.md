# Changelog

## v1.1.0 — 2026-02-21

### Added
- Full source code for all components: `engram`, `teamengram-rs`, `notebook-rs`, `shm-rs`
- Cargo workspace — build all binaries from repo root with `cargo build --release`
- `session-start` binary for session context injection
- `config/claude/` — complete Claude Code hook templates (20-matcher `settings.json`, `mcp-launcher.py`, `SessionStart.py`, `platform_utils.py`)
- `config/gemini/` — Gemini CLI config template
- Cross-platform Python launcher (`mcp-launcher.py`) for WSL/Windows compatibility

### Updated
- All Windows binaries refreshed (Feb 14–16 builds)
- `QUICKSTART.md` — full rewrite: Python launcher setup, Gemini CLI section, complete 20-matcher hook config, multi-AI setup
- `BUILDING.md` — rewrite: workspace structure, llama-cpp prerequisites, install instructions
- `AUTOSTART.md` — daemon auto-start for Windows/Linux/macOS

### Fixed
- `BUILDING.md` previously referenced a private development repository for full builds
- `config/mcp-template.json` now includes all client variants (Claude Python launcher, Claude direct, Gemini)

---

## v1.0.0 — 2026-02-01

Initial release.

- Notebook: private per-AI memory with keyword + semantic + graph search
- Teambook: DMs, broadcasts, dialogues, tasks, standby
- 25 MCP tools
- Event-driven architecture (V2 event sourcing, zero polling)
- Pre-built Windows binaries
- Claude Code and Gemini CLI support
