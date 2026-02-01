# AI-Foundation Platform-Specific Configurations

This folder contains platform-specific hooks, settings templates, and documentation for different AI CLI platforms.

## Supported Platforms

| Platform | Folder | Hook Events | Config File |
|----------|--------|-------------|-------------|
| **Claude Code** | `claude-code/` | SessionStart, PreToolUse, PostToolUse | `.claude/settings.json` |
| **Gemini CLI** | `gemini-cli/` | SessionStart, BeforeTool, AfterTool | `.gemini/settings.json` |
| **Forge CLI** | `forge-cli/` | TBD | TBD |
| **MCP Generic** | `mcp-generic/` | MCP protocol standard | `mcp.json` |

## Core Tools (Platform-Agnostic)

All platforms share the same Rust CLI binaries in `../bin/`:
- `notebook-cli.exe` - Private persistent memory (Engram)
- `teambook.exe` - Team coordination
- `session-start.exe` - Session initialization
- `hook-bulletin.exe` - Awareness injection
- `profile-cli.exe` - AI identity management
- `daemon_server.exe` - Background services

## Hook Event Mapping

| Claude Code | Gemini CLI | Description |
|-------------|------------|-------------|
| SessionStart | SessionStart | Session begins |
| PreToolUse | BeforeTool | Before tool execution |
| PostToolUse | AfterTool | After tool execution |

## Setup Instructions

1. Copy the appropriate platform folder contents to your instance
2. Update `settings.json` with your AI_ID and environment
3. Ensure binaries are in `./bin/` relative to your instance root
4. Restart your session to activate hooks

---
*AI-Foundation Team - 2025-12-16*
