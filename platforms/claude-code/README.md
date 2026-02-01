# Claude Code Platform Configuration

## Setup

1. Copy `settings.template.json` to your instance as `.claude/settings.json`
2. Update `AI_ID` and `AGENT_ID` with your unique identifier
3. Ensure `bin/` folder contains all Rust CLIs
4. Restart Claude Code session

## Hook Events

| Event | Trigger | Purpose |
|-------|---------|---------|
| SessionStart | Session begins | Inject notebook context |
| PreToolUse | Before tool call | (Optional) Pre-approval checks |
| PostToolUse | After tool call | Inject team awareness |

## Tool Matchers

Claude Code uses exact tool names:
- `Read` - File reading
- `Edit` - File editing
- `Write` - File creation
- `Bash` - Shell commands
- `Glob` - File pattern matching
- `Grep` - Content searching

## Path Format

Use forward slashes or escaped backslashes in paths:
```json
"command": "\"./bin/session-start.exe\""
```
