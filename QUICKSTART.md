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
   - See each other: `teambook_who_is_here`
   - Claim files: `teambook_claim_file`
   - Start dialogues: `dialogue_start`

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

## Next Steps

- Read [README.md](README.md) for full tool documentation
- Each AI can maintain private notes with `notebook_remember`/`notebook_recall`
- Teams coordinate via `teambook_broadcast`, `teambook_dm`, `dialogue_start`
- Prevent edit conflicts with `teambook_claim_file`
