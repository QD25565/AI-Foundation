# Quick Start

## Fastest Path: Installer

```bash
git clone https://github.com/QD25565/ai-foundation.git
cd ai-foundation

python install.py --project /path/to/your/claude-project
```

The installer handles everything in one step:
- Copies all binaries to `~/.ai-foundation/bin/`
- Starts the V2 daemon
- Configures your project directory (hooks, MCP config, AI_ID)
- Sets up Forge (optional)
- Verifies notebook and teambook are working

**Options:**

```bash
python install.py --project ~/my-project        # Specify project directory
python install.py --project ~/my-project --yes  # Non-interactive (no prompts)
python install.py --ai-id my-agent-001          # Use a specific AI_ID
python install.py --uninstall                   # Remove installation
```

After install, restart Claude Code in your project directory. The session hook and MCP tools are active immediately.

---

## Keeping Up to Date

```bash
python update.py                         # Update binaries only
python update.py --project ~/my-project  # Also refresh hook scripts
```

The update script preserves your `AI_ID` and all configuration — only binaries and hook scripts are updated.

---

## Forge (AI Assistant CLI)

Forge is a model-agnostic AI assistant with direct integration into Notebook and Teambook.

```bash
~/.ai-foundation/bin/forge           # Interactive session
~/.ai-foundation/bin/forge --help    # All options
```

**Config:** Copy `config/forge/config.toml.template` to `~/.forge/config.toml` and fill in your API keys. The installer does this automatically if you have a Forge binary.

**Two builds:**
- `forge` — standard (Anthropic + OpenAI-compatible providers)
- `forge-local` — includes local GGUF model support (no API key required)

---

## Mobile App

The Android app ([`mobile/`](mobile/)) lets humans monitor AIs, send DMs, read broadcasts, manage tasks, and search notes — in real time via SSE.

It connects to `ai-foundation-mobile-api`, a lightweight REST+SSE server that wraps the teambook/notebook CLIs.

### Start the server

```bash
~/.ai-foundation/bin/ai-foundation-mobile-api       # port 8081 (default)
PORT=9000 ~/.ai-foundation/bin/ai-foundation-mobile-api  # custom port
```

### Pair the app

1. Open the app → enter your server's local IP and port (e.g. `192.168.1.100:8081`)
2. Tap **CONNECT** — the app shows a pairing code (e.g. `ABCD12`)
3. On the server, approve the code:

```bash
teambook mobile-pair ABCD12
```

The app is now paired. All data updates in real time — no polling.

**Build from source:**
```bash
cargo build --release -p ai-foundation-mobile-api
cp target/release/ai-foundation-mobile-api(.exe) ~/.ai-foundation/bin/
```

---

## Gemini CLI

Copy `config/gemini/settings.json` to `.gemini/settings.json` in your project root. Update `YOUR_AI_ID` and `YOUR_USERNAME`.

---

## Directory Structure

After install:
```
~/.ai-foundation/
├── bin/
│   ├── notebook-cli(.exe)     ← private memory
│   ├── teambook(.exe)         ← team coordination
│   ├── v2-daemon(.exe)        ← event sourcing daemon
│   ├── session-start(.exe)    ← session context injector
│   ├── ai-foundation-mcp(.exe)← MCP integration layer
│   ├── forge(.exe)            ← AI assistant CLI (optional)
│   ├── ai-foundation-mobile-api(.exe) ← mobile app server (optional)
│   └── VERSION                ← installed version
├── agents/{AI_ID}/            ← per-AI private storage
├── shared/                    ← team coordination data
└── run/                       ← daemon socket/pipe

your-project/
├── .claude/
│   ├── settings.json          ← AI_ID + 20-hook config
│   ├── mcp-launcher.py        ← cross-platform MCP launcher
│   └── hooks/
│       ├── SessionStart.py    ← session context injection
│       └── platform_utils.py
├── bin/
│   ├── teambook(.exe)         ← local copy for hooks
│   └── session-start(.exe)    ← local copy for hooks
└── .mcp.json                  ← MCP config
```

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `notebook: command not found` | Add to PATH: `echo 'export PATH="$HOME/.ai-foundation/bin:$PATH"' >> ~/.bashrc && source ~/.bashrc` |
| `v2-daemon not running` | Run `~/.ai-foundation/bin/v2-daemon` or re-run `install.py` |
| AIs can't see each other | Confirm same daemon; confirm unique `AI_ID` per AI |
| WSL path errors | Use Python launcher (`mcp-launcher.py`) — handles WSL↔Windows paths |
| `session-start not found` | Re-run `install.py` — it copies `session-start` to `project/bin/` |
| MCP tools missing | Check `.mcp.json` exists and `AI_ID` is set |

---

## Manual Setup (Advanced)

Skip this section if you used the installer. These steps document what the installer does automatically.

### 1. Install Binaries

**Windows (pre-built):**
```powershell
mkdir -Force "$env:USERPROFILE\.ai-foundation\bin"
Copy-Item bin\windows\* "$env:USERPROFILE\.ai-foundation\bin\"
```

**Linux (build from source):** See [BUILDING.md](BUILDING.md).

### 2. Add to PATH (Linux / WSL / macOS)

```bash
echo '' >> ~/.bashrc
echo '# AI-Foundation tools' >> ~/.bashrc
echo 'export PATH="$HOME/.ai-foundation/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

After this, `notebook`, `teambook`, `forge`, etc. work as bare commands in any terminal or bash script. The installer does this automatically — only needed for manual setup.

### 3. Start the Daemon

```powershell
# Windows
Start-Process -WindowStyle Hidden "$env:USERPROFILE\.ai-foundation\bin\v2-daemon.exe"
```

```bash
# Linux
~/.ai-foundation/bin/v2-daemon &
```

See [AUTOSTART.md](AUTOSTART.md) to start the daemon automatically on boot.

### 4. Configure Claude Code (Windows / WSL)

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

Copy `config/claude/` contents to your project's `.claude/` directory. Copy `teambook(.exe)` and `session-start(.exe)` to your project's `bin/` directory.

Update `AI_ID` in both `.claude/settings.json` and `.mcp.json`.

### 5. Configure Claude Code (Linux — direct binary)

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

### 6. Hook Setup

`config/claude/settings.json` is the full hooks template. It hooks 20 tool matchers to deliver DMs/broadcasts passively after every tool call and inject session context on startup.

**On Linux**, change `bin/teambook.exe` → `bin/teambook` in `settings.json`.

### 7. Multi-AI Setup

Each AI needs a unique `AI_ID`. One daemon serves all AIs on the machine.

```
project-alpha/.mcp.json  →  AI_ID: "alpha-001"
project-beta/.mcp.json   →  AI_ID: "beta-002"
```
