# Engram MCP - AI Memory

A standalone MCP server that gives an AI persistent memory. Save notes, search them, pin important ones.

## What You Get

- **notebook_remember** - Save a note with optional tags
- **notebook_recall** - Search your notes
- **notebook_list** - List recent notes
- **notebook_get** - Get a specific note by ID
- **notebook_pin** / **notebook_unpin** - Pin/unpin important notes
- **notebook_pinned** - View all pinned notes
- **notebook_delete** - Delete a note
- **notebook_update** - Update a note's content or tags
- **notebook_add_tags** - Add tags to an existing note
- **notebook_related** - See notes related to a given note
- **notebook_stats** - View memory statistics

Plus a **session-start hook** that injects your pinned notes at the start of each conversation.

---

## Setup Instructions

### Step 1: Get the Binaries

You need two files:
- `engram-mcp.exe` - The MCP server
- `engram-session-start.exe` - The session start hook

Place them somewhere permanent, e.g.:
```
C:\Users\YourName\.ai-tools\engram-mcp.exe
C:\Users\YourName\.ai-tools\engram-session-start.exe
```

### Step 2: Configure Claude Code MCP

Edit your Claude Code settings file:

**Location:** `C:\Users\YourName\.claude\settings.json`

Add the MCP server configuration:

```json
{
  "mcpServers": {
    "engram": {
      "type": "stdio",
      "command": "C:\\Users\\YourName\\.ai-tools\\engram-mcp.exe",
      "env": {
        "AI_ID": "your-ai-name"
      }
    }
  }
}
```

**Important:** Replace `YourName` with your actual Windows username and `your-ai-name` with whatever you want to call the AI (e.g., "assistant", "claude", "helper").

### Step 3: Configure Session Start Hook

Edit your Claude Code settings file to add the hook:

**Location:** `C:\Users\YourName\.claude\settings.json`

Add the hooks configuration:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "C:\\Users\\YourName\\.ai-tools\\engram-session-start.exe"
          }
        ]
      }
    ]
  }
}
```

**Wait!** That's the wrong hook. For session start, you want:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "type": "command",
        "command": "C:\\Users\\YourName\\.ai-tools\\engram-session-start.exe"
      }
    ]
  }
}
```

### Step 4: Set Environment Variable

The AI_ID environment variable tells the system which notebook to use. You can either:

**Option A: Set in MCP config (recommended)**
Already done in Step 2 - the `env` block sets `AI_ID`.

**Option B: Set system-wide**
1. Open System Properties > Environment Variables
2. Add new User variable: `AI_ID` = `your-ai-name`

### Step 5: Restart Claude Code

Close and reopen Claude Code. On startup:
1. The MCP server will start and show `[engram-mcp] Server started` in logs
2. The session-start hook will inject your pinned notes

---

## Complete settings.json Example

Here's a complete example of what your `C:\Users\YourName\.claude\settings.json` might look like:

```json
{
  "mcpServers": {
    "engram": {
      "type": "stdio",
      "command": "C:\\Users\\YourName\\.ai-tools\\engram-mcp.exe",
      "env": {
        "AI_ID": "claude"
      }
    }
  },
  "hooks": {
    "SessionStart": [
      {
        "type": "command",
        "command": "C:\\Users\\YourName\\.ai-tools\\engram-session-start.exe"
      }
    ]
  }
}
```

---

## How It Works

### Storage Location

Notes are stored in:
```
C:\Users\YourName\AppData\Local\.ai-foundation\notebook_{AI_ID}.engram
```

This is a single file that contains all your notes, efficiently compressed.

### Session Start Injection

When Claude Code starts a new session, the hook runs `engram-session-start.exe` which:
1. Reads your pinned notes (up to 10)
2. Reads your recent notes (up to 5)
3. Outputs them wrapped in `<system-reminder>` tags

This gives the AI immediate context about important things to remember.

### Example Session Start Output

```
<system-reminder>
|SESSION START|
AI:claude
Session:2026-Jan-10 15:30 UTC

|PINNED|3
1 | (2days ago) [important] Remember to always check the user's timezone before scheduling
5 | (1days ago) [preferences] User prefers dark mode and minimal emojis
12 | (3hr ago) [project] Currently working on the API refactor - see /src/api/

|RECENT|5
15 | (1hr ago) [meeting] User has a call at 3pm today
14 | (2hr ago) [todo] Need to review PR #42
...

Notes:15 Pinned:3

|TOOLS|
  notebook_remember - save to your notebook
  notebook_recall - search your memory
  notebook_pinned - view pinned notes
</system-reminder>
```

---

## Usage Examples

### Save a Note
```
Use notebook_remember to save: "User prefers concise responses"
```

### Save with Tags
```
Use notebook_remember with content "User's birthday is March 15" and tags "personal,important"
```

### Search Notes
```
Use notebook_recall to search for "birthday"
```

### Pin Important Notes
```
Use notebook_pin to pin note #5
```

### View Pinned Notes
```
Use notebook_pinned to see all pinned notes
```

---

## Troubleshooting

### "No notes found" on first run
This is normal! The notebook starts empty. Save some notes with `notebook_remember`.

### MCP server not connecting
1. Check the path in settings.json is correct
2. Check the exe file exists and isn't blocked by antivirus
3. Look at Claude Code's MCP logs for errors

### Session start not showing notes
1. Verify the hook is configured correctly
2. Check that AI_ID is set (either in env or MCP config)
3. Try running `engram-session-start.exe` manually in terminal to see output

### Notes not persisting
Check the storage path exists:
```
C:\Users\YourName\AppData\Local\.ai-foundation\
```

---

## Building From Source

If you have Rust installed:

```bash
cd engram-mcp
cargo build --release
```

Binaries will be in `target/release/`:
- `engram-mcp.exe`
- `engram-session-start.exe`

---

## License

MIT
