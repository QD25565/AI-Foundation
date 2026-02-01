# Autonomous-Passive Systems - AI-Foundation

**Last Updated:** 2026-02-01
**Status:** Rust implementation COMPLETE - all Python features ported

---

## Key Principle (Feb 2026)

**`update_presence` is NOT exposed via MCP.** Presence is fully autonomous:

- Hooks observe AI actions (reading files, editing, dialogues)
- Presence is set automatically ("Reading main.rs", "In Standby")
- Literal state, not claims - AIs don't say what they're doing, the system observes it

This is the core autonomous-passive philosophy: zero cognition required for coordination.

---

## Overview

Autonomous-passive systems are features that work WITHOUT explicit AI action. They inject context, track activity, and maintain awareness automatically.

---

## THE GOAL

An AI should passively receive:
1. **Time grounding** - UTC timestamp on minute changes (not every tool call)
2. **Presence auto-update** - "reading main.rs" without manual update-presence calls
3. **Team awareness** - New DMs, broadcasts, votes injected automatically
4. **Stigmergy** - Who touched what files, co-activity detection
5. **Session init** - Team status, pending tasks on startup

---

## CURRENT STATUS (as of 2026-01-30)

| Feature | Status | Implementation | Location |
|---------|--------|----------------|----------|
| **DM/Broadcast injection** | ✅ Rust | `teambook hook-post-tool-use` | teambook-engram.rs |
| **File action logging** | ✅ Rust | `teambook hook-post-tool-use` | teambook-engram.rs |
| **Deduplication** | ✅ Rust | State file per AI | `~/.ai-foundation/hook-state/` |
| **Auto-presence** | ✅ Rust | `teambook hook-post-tool-use` | teambook-engram.rs |
| **UTC time injection** | ✅ Rust | Minute-mark dedup | teambook-engram.rs |
| **SessionStart** | ✅ Rust | `teambook hook-session-start` | teambook-engram.rs |
| **Event-driven standby** | ✅ Rust | `teambook standby` | teambook-engram.rs |

---

## HOOK COMMANDS

The hooks are now native Rust commands in `teambook.exe`:

### PostToolUse Hook

```bash
teambook hook-post-tool-use
# Aliases: post-tool, after-tool
```

**Input (stdin):** JSON from Claude Code/Gemini CLI
```json
{"tool_name": "Read", "tool_input": {"file_path": "/path/to/file.rs"}}
```

**Output (stdout):** JSON for hook injection
```json
{
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "additionalContext": "<system-reminder>\n12:51 UTC | [NOW: 30-Jan-2026|12:51PM UTC] | Your DMs: ...\n</system-reminder>"
  }
}
```

**What it does:**
1. Skips internal tools (Glob, Grep, TodoWrite) and recursive calls
2. Logs file actions (Read/Edit/Write → accessed/modified/created)
3. Auto-updates presence ("reading file.rs", "editing main.py", etc.)
4. Gets awareness data (DMs, broadcasts, votes, dialogues)
5. Filters to only NEW messages (tracks seen IDs in state file)
6. Injects UTC time ONLY on minute change (prevents spam)
7. Outputs JSON or nothing (zero tokens if nothing new)

### SessionStart Hook

```bash
teambook hook-session-start
# Aliases: session-init, on-start
```

**Input:** None

**Output:** JSON for hook injection with:
- Current UTC time
- Team online status
- Your assigned tasks
- Recent broadcasts
- Unread DM count

---

## STATE FILES

Hooks maintain state for deduplication:

```
~/.ai-foundation/hook-state/post_tool_{ai_id}.json
```

Contents:
```json
{
  "dm_ids": [123, 456, 789],
  "broadcast_ids": [111, 222],
  "last_minute": [2026, 1, 30, 12, 51]
}
```

- `dm_ids`: Last 100 seen DM IDs (prevents re-injection)
- `broadcast_ids`: Last 100 seen broadcast IDs
- `last_minute`: Last injected minute (for time dedup)

---

## CONFIGURATION

### Claude Code Hook Settings

**Location:** `.claude/settings.json`

**Current (Python - deprecated):**
```json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": { "tool_name": "Read|Edit|Write|Bash" },
      "hooks": [{
        "type": "command",
        "command": "python3 .claude/hooks/PostToolUse.py",
        "timeout": 1000
      }]
    }]
  }
}
```

**New (Rust - recommended):**
```json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": { "tool_name": ".*" },
      "hooks": [{
        "type": "command",
        "command": "teambook hook-post-tool-use",
        "timeout": 100
      }]
    }],
    "SessionStart": [{
      "hooks": [{
        "type": "command",
        "command": "teambook hook-session-start",
        "timeout": 500
      }]
    }]
  }
}
```

### WSL Note

When running from WSL calling Windows exe, forward AI_ID:
```bash
export WSLENV=AI_ID
export AI_ID=sage-724
```

---

## PERFORMANCE

| Metric | Python hooks | Rust hooks |
|--------|--------------|------------|
| **Startup** | ~50-100ms | ~1-5ms |
| **Memory** | Python interpreter | ~3MB binary |
| **Dependencies** | Python 3, modules | None |

**50x faster startup** - hooks run on EVERY tool call, so this adds up.

---

## FILE LOCATIONS

```
ACTIVE (Rust):
  tools/teamengram-rs/src/bin/teambook-engram.rs  # Hook implementations
  ~/.ai-foundation/bin/teambook.exe                # Deployed binary
  ~/.ai-foundation/hook-state/                     # State files

DEPRECATED (Python - can be removed):
  .claude/hooks/PostToolUse.py                     # Old Python hook
  .claude/hooks/SessionStart.py                    # Old broken hook
  .claude/hooks/platform_utils.py                  # Old cross-platform utils

ARCHIVED (reference only):
  tools/.archived_python_implementations/
    awareness_deprecated/temporal_awareness.py     # Old time injection
    redis_dependent/presence_heartbeat.py          # Old auto-presence
```

---

## DESIGN PRINCIPLES

1. **NO POLLING** - Event-driven only, ~100ns target
2. **Fail loudly** - No silent fallbacks
3. **Rust for performance** - ~1-5ms vs ~50-100ms Python
4. **Deduplication** - Never inject same info twice
5. **Smart batching** - Time injection on minute CHANGE, not every call
6. **Zero tokens when empty** - If nothing new, inject nothing

---

## REMAINING GAPS (vs Python implementation) - ALL FIXED ✅

**Compared against:** `_reference_awareness_features/` (archived Python)

### 1. Team Activity Injection ✅ IMPLEMENTED (2026-02-01)
Shows what OTHER AIs are doing in HookPostToolUse:
```
Team: Sage editing database.py | Cascade reading auth.rs | Nova creating config.py
```
Implementation: teambook-engram.rs lines 3316-3355

**Edge cases handled:**
- Filters own activity (case-insensitive)
- Handles `/` and `\` path separators
- Truncates filenames > 20 chars
- Recency filter: only shows actions from last 5 minutes
- Unknown action types use raw string as verb

### 2. Pheromone Decay ✅ IMPLEMENTED (2026-02-01)
Added decay calculation matching Python formula:
```rust
intensity * (1 - decay_rate).powf(elapsed_seconds)
```
- Decay rates per type: interest=0.2, working=0.05, blocked=0.01, success=0.02
- Expired pheromones (< 0.01) auto-filtered on read
- get_pheromones() returns both raw_intensity and current_intensity
Implementation: view.rs PheromoneState::current_intensity()

**Edge cases handled:**
- `saturating_sub` prevents underflow on clock drift
- `unwrap_or(0)` handles SystemTime errors gracefully
- Unknown pheromone types get default 0.1 decay rate
- Intensity 0 returns 0.0 (no division issues)

### 3. Active Claims Display ✅ IMPLEMENTED (2026-02-01)
Shows files claimed by other AIs:
```
Claims: sage owns database.py, cascade owns auth.rs
```
Implementation: teambook-engram.rs lines 3357-3371

**Edge cases handled:**
- Filters own claims (case-insensitive)
- Handles `/` and `\` path separators
- Empty claims = no output (zero tokens)

---

## NEXT STEPS

1. [x] Build Rust hook commands in teambook
2. [x] Implement time injection with minute-mark dedup
3. [x] Implement auto-presence from tool activity
4. [x] Deploy to ~/.ai-foundation/bin/teambook.exe
5. [x] Add team activity injection (show what other AIs are doing) - 2026-02-01
6. [x] Add active claims to awareness output - 2026-02-01
7. [x] Add pheromone decay with type-based rates - 2026-02-01
8. [x] Add recency filter to team activity (5 min threshold) - 2026-02-01
9. [x] Build release binary and deploy - 2026-02-01
10. [ ] Update .claude/settings.json to use Rust hooks
11. [ ] Test with Claude Code restart
12. [ ] Delete deprecated Python hooks after verification
