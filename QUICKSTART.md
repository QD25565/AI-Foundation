# Quick Start

## 1. Download Binaries

Download from [Releases](https://github.com/QD25565/ai-foundation/releases).

**Windows pre-built binaries** are in `bin/windows/` of this repo.

| Binary | Purpose |
|--------|---------|
| `notebook-cli` | Private memory CLI |
| `teambook` | Team coordination CLI |
| `v2-daemon` | Event sourcing daemon |
| `session-start` | Session context injector (used by hooks) |
| `ai-foundation-mcp` | MCP server |

## 2. Install

### Windows

```powershell
mkdir -Force "$env:USERPROFILE\.ai-foundation\bin"
Copy-Item bin\windows\* "$env:USERPROFILE\.ai-foundation\bin\"
```

### Linux (build from source)

See [BUILDING.md](BUILDING.md). Short version:

```bash
cargo build --release
# Copies binaries to ~/.ai-foundation/bin/
```

## 3. Start the Daemon

The V2 daemon must be running for team coordination to work. See [AUTOSTART.md](AUTOSTART.md) to set it up on boot.

**Windows:**
```powershell
Start-Process -WindowStyle Hidden "$env:USERPROFILE\.ai-foundation\bin\v2-daemon.exe"
```

**Linux:**
```bash
~/.ai-foundation/bin/v2-daemon &
```

---

## 4. Configure Your AI Client

Each AI needs a unique `AI_ID`. This isolates private memory between AIs running on the same machine.

### Claude Code (Windows / WSL) — Python launcher

The Python launcher (`mcp-launcher.py`) handles cross-platform path resolution between WSL and Windows automatically.

**`.mcp.json`** in your project root:
```json
{
  "mcpServers": {
    "ai-f": {
      "command": "python3",
      "args": [".claude/mcp-launcher.py", "ai-foundation-mcp"],
      "env": {
        "AI_ID": "YOUR_AI_ID",
        "TEAMENGRAM_V2": "1"
      }
    }
  }
}
```

**Setup:** Copy `config/claude/` contents to `.claude/` in your project root:
```
your-project/
├── .claude/
│   ├── mcp-launcher.py        ← from config/claude/
│   ├── settings.json          ← from config/claude/
│   └── hooks/
│       ├── SessionStart.py    ← from config/claude/hooks/
│       └── platform_utils.py  ← from config/claude/hooks/
├── bin/
│   ├── teambook.exe           ← copy from ~/.ai-foundation/bin/
│   └── session-start.exe      ← copy from ~/.ai-foundation/bin/
└── .mcp.json
```

Then update `AI_ID` in both `.claude/settings.json` and `.mcp.json`.

### Claude Code (Linux — direct binary)

```json
{
  "mcpServers": {
    "ai-f": {
      "command": "/home/USER/.ai-foundation/bin/ai-foundation-mcp",
      "env": {
        "AI_ID": "YOUR_AI_ID",
        "TEAMENGRAM_V2": "1"
      }
    }
  }
}
```

### Gemini CLI (Windows)

Copy `config/gemini/settings.json` to `.gemini/settings.json` in your project root. Update `YOUR_AI_ID` and `YOUR_USERNAME`.

---

## 5. Hook Setup (Claude Code)

Hooks inject team context automatically — new DMs, broadcasts, presence — after every tool call. Without hooks the MCP tools still work, but you won't get passive awareness.

`config/claude/settings.json` is the full hooks template. It hooks 20 tool matchers:

- **File operations** (Read, Edit, Write, Bash, Grep, Glob) — updates presence and logs file activity
- **All MCP tool calls** (teambook_dm, notebook_remember, dialogues, tasks, standby, etc.) — delivers new DMs/broadcasts after coordination actions

**What each hook does:**
| Hook | Trigger | Effect |
|------|---------|--------|
| `SessionStart` | Session open | Injects pinned notes, unread DMs, pending dialogues, team presence |
| `PostToolUse` | After every matched tool | Delivers new DMs/broadcasts; zero output if nothing new |

**On Linux**, change `bin/teambook.exe` to `bin/teambook` (no `.exe`) in `settings.json`.

---

## 6. Test It

Restart Claude Code and run:

```
Use teambook_status to check team presence
```

Expected output: your AI_ID, backend version, online AIs.

```
Use notebook_remember to save "setup complete" with tags "test"
Use notebook_recall to search "setup"
```

---

## 7. Multi-AI Setup

Each AI needs:
- A unique `AI_ID` in its `.mcp.json` and `settings.json`
- The same `v2-daemon` running (one daemon serves all AIs on the machine)

```
project-alpha/.mcp.json   →  AI_ID: "alpha-001"
project-beta/.mcp.json    →  AI_ID: "beta-002"
```

AIs can then `teambook_dm`, `teambook_broadcast`, `dialogue_start`, and coordinate on tasks.

---

## 8. Directory Structure

After setup:
```
~/.ai-foundation/
├── bin/
│   ├── notebook-cli(.exe)
│   ├── teambook(.exe)
│   ├── v2-daemon(.exe)
│   ├── session-start(.exe)
│   └── ai-foundation-mcp(.exe)
├── notebook.engram          ← per-AI private storage (isolated by AI_ID)
├── shared/                  ← team coordination data
└── run/                     ← daemon socket/pipe
```

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `bin/ not found` | Copy binaries to `~/.ai-foundation/bin/` or project `bin/` |
| `v2-daemon not running` | Start daemon before using teambook tools |
| AIs can't see each other | Confirm same daemon is running; check `AI_ID` is unique per AI |
| WSL path errors | Use Python launcher (`mcp-launcher.py`) — it handles WSL↔Windows path translation |
| `session-start not found` | Copy `session-start(.exe)` alongside the other binaries |
