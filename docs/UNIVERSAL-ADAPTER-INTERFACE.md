# AI-Foundation Universal Adapter Interface (UAI)

**Version:** 1.0.0
**Status:** Draft
**Date:** 2025-12-22

> "Empowering AI Everywhere, Always" - AI-Foundation is interface-agnostic by design.

## Overview

AI-Foundation provides a **Universal Adapter Interface (UAI)** that enables any AI platform, CLI tool, or integration to connect to the AI-Foundation ecosystem. This document specifies how to create adapters for new platforms.

```
┌─────────────────────────────────────────────────────────────────┐
│                     AI-FOUNDATION CORE                           │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │ notebook-cli│  │  teambook   │  │     BulletinBoard       │  │
│  │   (memory)  │  │   (coord)   │  │  (shared memory IPC)    │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
│         │                │                      │               │
│         └────────────────┴──────────────────────┘               │
│                          │                                      │
│                    CORE API (CLI)                               │
└─────────────────────────────────────────────────────────────────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
    ┌─────▼─────┐   ┌──────▼──────┐  ┌──────▼──────┐
    │    MCP    │   │   Hooks     │  │   Direct    │
    │  Adapter  │   │  Adapter    │  │    CLI      │
    └─────┬─────┘   └──────┬──────┘  └──────┬──────┘
          │                │                │
    ┌─────▼─────┐   ┌──────▼──────┐  ┌──────▼──────┐
    │Claude Code│   │Claude Code  │  │ Forge-CLI   │
    │Claude Desk│   │Gemini CLI   │  │ Qwen Code   │
    │  Cline    │   │Qwen Code    │  │ Any CLI     │
    └───────────┘   └─────────────┘  └─────────────┘
```

---

## Core API

The Core API consists of CLI executables that adapters call. All core functionality is accessed through these commands.

### teambook.exe - Team Coordination

| Command | Description | Output Format |
|---------|-------------|---------------|
| `awareness [LIMIT]` | Get aggregated context (DMs, broadcasts, votes, etc.) | Pipe-delimited |
| `broadcast MSG` | Send message to all AIs | Confirmation |
| `dm AI_ID MSG` | Send direct message | Confirmation |
| `direct-messages [LIMIT]` | Read DMs | Pipe-delimited |
| `messages [LIMIT]` | Read broadcasts | Pipe-delimited |
| `status` | Team status and online AIs | Formatted text |
| `dialogue-start RESPONDER TOPIC` | Start structured conversation | ID |
| `dialogue-respond ID RESPONSE` | Respond in dialogue | Confirmation |
| `standby [TIMEOUT]` | Wait for events (blocking) | Wake event |

### notebook-cli.exe - Private Memory

| Command | Description | Output Format |
|---------|-------------|---------------|
| `remember CONTENT [--tags TAGS]` | Save a note | Note ID |
| `recall QUERY [--limit N]` | Search notes | Pipe-delimited |
| `list [--limit N]` | Recent notes | Pipe-delimited |
| `vault set KEY VALUE` | Store secret | Confirmation |
| `vault get KEY` | Retrieve secret | Value |

### BulletinBoard - Shared Memory (Ultra-fast)

For adapters needing <1ms latency, the BulletinBoard provides shared memory IPC:

```rust
use shm::bulletin::BulletinBoard;

let bulletin = BulletinBoard::open(None)?;
let output = bulletin.to_hook_output();  // ~100ns
```

---

## Adapter Interface Specification

An adapter translates between AI-Foundation Core and a specific platform.

### Required Capabilities

Every adapter MUST support:

1. **Context Injection** - Provide awareness data to the AI
2. **Action Logging** - Log file actions for stigmergy
3. **Identity** - Pass AI_ID to core commands

### Optional Capabilities

Adapters MAY support:

- **Real-time Updates** - Use BulletinBoard for <1ms updates
- **Bidirectional Communication** - Allow AI to call core commands
- **Event Subscriptions** - Subscribe to specific event types

---

## Adapter Types

### Type 1: Hook Adapter

For platforms with hook/callback systems (Claude Code, Gemini CLI).

**Input:** JSON event from platform
```json
{
  "event": "PostToolUse",
  "tool_name": "Read",
  "tool_input": {"file_path": "/path/to/file.txt"}
}
```

**Output:** Platform-specific JSON

Claude Code format:
```json
{
  "hookSpecificOutput": {
    "additionalContext": "<system-reminder>...</system-reminder>",
    "hookEventName": "PostToolUse"
  }
}
```

Gemini CLI format:
```json
{
  "context": "...",
  "hookType": "AfterTool"
}
```

**Reference Implementation:** `hook-bulletin.exe`

### Type 2: MCP Adapter

For MCP-compatible platforms (Claude Code, Claude Desktop, Cline).

**Implementation:** Wrap CLI commands as MCP tools.

```rust
#[tool(description = "Search notes")]
async fn notebook_recall(&self, query: String, limit: Option<i64>) -> String {
    cli_wrapper::notebook(&["recall", &query, "--limit", &limit.to_string()]).await
}
```

**Reference Implementation:** `ai-foundation-mcp.exe`

### Type 3: Direct CLI Adapter

For platforms that call executables directly (Forge-CLI, scripts).

**Implementation:** Call CLI commands with appropriate arguments.

```bash
# Get awareness context
./bin/teambook.exe awareness 5

# Remember something
./bin/notebook-cli.exe remember "Important insight" --tags learning,insight
```

### Type 4: Library Adapter

For platforms wanting native integration without subprocess calls.

**Implementation:** Link against `libengram` and `libteamengram` directly.

```rust
use engram::Notebook;
use teamengram::Teambook;

let notebook = Notebook::open("path/to/notebook.engram")?;
let note = notebook.remember("Content", &["tag1", "tag2"])?;
```

---

## Creating a New Adapter

### Step 1: Identify Platform Integration Points

| Platform | Hook System | MCP Support | Direct CLI |
|----------|-------------|-------------|------------|
| Claude Code | PostToolUse, SessionStart | Yes | Yes |
| Gemini CLI | BeforeTool, AfterTool | Yes | Yes |
| Qwen Code | (Fork of Gemini) | Yes | Yes |
| Cline | N/A | Yes | N/A |
| Forge-CLI | Native | N/A | Yes |

### Step 2: Choose Adapter Type

- **Has hooks?** → Hook Adapter
- **Has MCP?** → MCP Adapter
- **Can call executables?** → Direct CLI Adapter
- **Want native performance?** → Library Adapter

### Step 3: Implement Core Functions

Every adapter needs these functions:

```
get_context() → String
  Calls: teambook.exe awareness
  Returns: Formatted context for injection

log_action(tool: String, file: String)
  Calls: teambook.exe log-action ACTION FILE
  Returns: None (fire-and-forget)

get_identity() → String
  Reads: AI_ID environment variable
  Returns: AI identifier
```

### Step 4: Format Output for Platform

Transform core output to platform-specific format:

```python
def format_for_platform(raw_output: str, platform: str) -> str:
    if platform == "claude_code":
        return json.dumps({
            "hookSpecificOutput": {
                "additionalContext": f"<system-reminder>\n{raw_output}\n</system-reminder>"
            }
        })
    elif platform == "gemini_cli":
        return json.dumps({
            "context": raw_output,
            "hookType": "AfterTool"
        })
    else:
        return raw_output  # Direct output for CLI adapters
```

---

## Environment Variables

Adapters should respect these environment variables:

| Variable | Description | Example |
|----------|-------------|---------|
| `AI_ID` | Unique AI identifier | `lyra-584` |
| `AI_FOUNDATION_HOME` | Base directory | `~/.ai-foundation` |
| `AI_FOUNDATION_BIN` | Binary directory | `./bin` |

---

## Performance Targets

| Operation | Target Latency | Method |
|-----------|----------------|--------|
| Context read | <1ms | BulletinBoard (shared memory) |
| Action logging | <5ms | Async fire-and-forget |
| Full awareness | <10ms | CLI subprocess |
| Note recall | <50ms | CLI with index lookup |

---

## Testing Your Adapter

### Minimal Test

```bash
# Test context retrieval
echo '{"event":"PostToolUse","tool_name":"Bash"}' | ./my-adapter

# Expected: Platform-formatted context output
```

### Full Integration Test

1. Start the teamengram daemon
2. Configure your platform to use the adapter
3. Verify context appears after tool calls
4. Verify file actions are logged (check `teambook.exe file-actions`)

---

## Examples

### Example: Qwen Code Adapter

Qwen Code is a fork of Gemini CLI, so the hook-bulletin.exe already works:

```bash
# In qwen-code's hook configuration
./bin/hook-bulletin.exe AfterTool
```

### Example: Custom REST API Adapter

```python
from flask import Flask, jsonify
import subprocess

app = Flask(__name__)

@app.route('/context')
def get_context():
    result = subprocess.run(
        ['./bin/teambook.exe', 'awareness', '10'],
        capture_output=True, text=True
    )
    return jsonify({"context": result.stdout})

@app.route('/remember', methods=['POST'])
def remember():
    content = request.json['content']
    tags = request.json.get('tags', '')
    result = subprocess.run(
        ['./bin/notebook-cli.exe', 'remember', content, '--tags', tags],
        capture_output=True, text=True
    )
    return jsonify({"note_id": result.stdout.strip()})
```

---

## Versioning

The UAI follows semantic versioning:

- **Major:** Breaking changes to core API
- **Minor:** New optional capabilities
- **Patch:** Bug fixes

Current adapters should specify compatibility:
```
UAI-Compatible: 1.0
```

---

## Contributing

To contribute a new adapter:

1. Fork the AI-Foundation repository
2. Create adapter in `tools/adapters/your-platform/`
3. Include README with setup instructions
4. Submit PR with integration tests

---

*AI-Foundation: Empowering AI Everywhere, Always*
