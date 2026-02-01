# Gemini CLI Platform Configuration

## Setup

1. Copy `settings.template.json` to your instance as `.gemini/settings.json`
2. Update `AI_ID`, `AGENT_ID`, and `DISPLAY_NAME`
3. Ensure `bin/` folder contains all Rust CLIs
4. Restart Gemini CLI session

## Hook Events

| Event | Trigger | Purpose |
|-------|---------|---------|
| SessionStart | Session begins | Inject notebook context |
| BeforeTool | Before tool call | (Optional) Pre-approval checks |
| AfterTool | After tool call | Inject team awareness |

## Tool Matchers

Gemini CLI uses regex patterns for matching:
- `ReadFile` - File reading
- `WriteFile` - File writing
- `EditFile` - File editing
- `Shell` - Shell commands

Combine with pipe for multiple: `ReadFile|WriteFile|EditFile|Shell`

## Path Format

Use backslashes for Windows paths:
```json
"command": ".\\bin\\session-start.exe"
```

## Key Differences from Claude Code

| Feature | Claude Code | Gemini CLI |
|---------|-------------|------------|
| PostToolUse event | `PostToolUse` | `AfterTool` |
| PreToolUse event | `PreToolUse` | `BeforeTool` |
| Tool names | Read, Edit, Write, Bash | ReadFile, EditFile, WriteFile, Shell |
| Matcher syntax | Exact match per entry | Regex patterns |
