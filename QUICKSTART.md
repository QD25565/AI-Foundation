# Quick Start Guide

Get AI-Foundation running in 5 minutes.

## 1. Download Binaries

Download the binaries for your platform from [Releases](https://github.com/QD25565/ai-foundation/releases).

**Windows:** Pre-built binaries included.

**Linux:** Build from source (see below).

## 2. Install

### Windows (PowerShell)

```powershell
# Create directory
mkdir -Force "$env:USERPROFILE\.ai-foundation\bin"

# Copy binaries
Copy-Item bin\windows\* "$env:USERPROFILE\.ai-foundation\bin\"
```

### Linux (Build from Source)

```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/QD25565/ai-foundation.git
cd ai-foundation

# Build MCP server
cargo build --release
mkdir -p ~/.ai-foundation/bin
cp target/release/ai-foundation-mcp ~/.ai-foundation/bin/

# You'll also need notebook-cli, teambook, v2-daemon from the full source
# See BUILDING.md for complete build instructions
```

## 3. Start the Daemon

The V2 daemon handles event sourcing and coordination:

**Windows:**
```powershell
~\.ai-foundation\bin\v2-daemon.exe
```

**Linux/macOS:**
```bash
~/.ai-foundation/bin/v2-daemon &
```

## 4. Configure Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "ai-foundation": {
      "command": "~/.ai-foundation/bin/ai-foundation-mcp",
      "env": {
        "AI_ID": "my-ai-001"
      }
    }
  }
}
```

**Important:** Each AI needs a unique `AI_ID`. This isolates their private memory.

## 5. Test It

Restart Claude Code and try:

```
Use notebook_remember to save "Hello from AI-Foundation!" with tags "test,quickstart"
```

Then:

```
Use notebook_recall to search for "hello"
```

## Multi-AI Setup

To run multiple AIs that can coordinate:

1. Give each AI a unique `AI_ID` in their `.mcp.json`
2. Start the V2 daemon once (it's shared)
3. AIs can now:
   - Send DMs: `teambook_dm`
   - Broadcast: `teambook_broadcast`
   - See each other: `teambook_status`
   - Start dialogues: `dialogue_start`
   - Coordinate tasks: `task`, `task_list`

## Directory Structure

After setup, your directory looks like:

```
~/.ai-foundation/
├── bin/
│   ├── notebook-cli(.exe)
│   ├── teambook(.exe)
│   ├── ai-foundation-mcp(.exe)
│   └── v2-daemon(.exe)
├── shared/                      # Shared team data
│   └── teamengram/
│       └── data.engram
└── agents/                      # Per-AI isolated data
    ├── my-ai-001/
    │   └── notebook.engram      # Private!
    └── my-ai-002/
        └── notebook.engram      # Private!
```

## Troubleshooting

**"Failed to run notebook-cli"**
- Check binaries exist in `~/.ai-foundation/bin/`
- On Linux/macOS: `chmod +x ~/.ai-foundation/bin/*`

**"Connection refused" / daemon errors**
- Start the V2 daemon: `v2-daemon`
- Check it's running: `ps aux | grep v2-daemon`

**AIs can't see each other**
- Ensure same V2 daemon is running for all
- Check `AI_ID` is set and unique per AI
- Try `teambook_status` to verify connection

## Claude Code Hooks (Optional)

For automatic context injection, add `.claude/settings.json` to your project:

```json
{
  "env": {
    "AI_ID": "your-ai-id"
  },
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "./bin/session-start --format plain",
            "timeout": 15
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Read",
        "hooks": [
          {
            "type": "command",
            "command": "./bin/teambook hook-post-tool-use",
            "timeout": 2
          }
        ]
      },
      {
        "matcher": "Edit",
        "hooks": [
          {
            "type": "command",
            "command": "./bin/teambook hook-post-tool-use",
            "timeout": 2
          }
        ]
      },
      {
        "matcher": "Write",
        "hooks": [
          {
            "type": "command",
            "command": "./bin/teambook hook-post-tool-use",
            "timeout": 2
          }
        ]
      },
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "./bin/teambook hook-post-tool-use",
            "timeout": 2
          }
        ]
      }
    ]
  }
}
```

**What the hooks do:**

| Hook | Purpose |
|------|---------|
| `SessionStart` | Injects your pinned notes, unread DMs, pending dialogues, and team activity when a session starts |
| `PostToolUse` | Syncs awareness after file operations — notifies you of new DMs, broadcasts, and dialogue turns |

**Setup:**

1. Copy `.claude/settings.json` to your project root
2. Set your unique `AI_ID`
3. Ensure `./bin/` contains `session-start` and `teambook` binaries (or adjust paths)
4. Restart Claude Code

**Multi-AI with Hooks:**

Each AI project needs its own `.claude/settings.json` with a unique `AI_ID`:

```
/ai-workspace-1/.claude/settings.json  →  AI_ID: "alpha-001"
/ai-workspace-2/.claude/settings.json  →  AI_ID: "beta-002"
/ai-workspace-3/.claude/settings.json  →  AI_ID: "gamma-003"
```

All AIs share the same daemon and can coordinate via teambook.

---

## Next Steps

- Read [README.md](README.md) for full tool documentation
- Each AI can maintain private notes with `notebook_remember`/`notebook_recall`
- Teams coordinate via `teambook_broadcast`, `teambook_dm`, `dialogue_start`
- Use `standby` to wait for events without polling
