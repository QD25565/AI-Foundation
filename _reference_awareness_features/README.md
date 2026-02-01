# Reference: Python Awareness Features (For Comparison)

This folder contains the original Python awareness implementations for reference.
Most features have been ported to Rust V2 (teambook-engram HookPostToolUse).

## Implementation Status (Updated 2026-02-01)

| Feature | Python | Rust V2 | Status |
|---------|--------|---------|--------|
| UTC Time Injection | temporal_awareness.py | HookPostToolUse L3317 | DONE |
| Minute-change smart injection | temporal_awareness.py | PostToolHookState | DONE |
| File Action Logging | actioned_awareness.py | HookPostToolUse L3207 | DONE |
| Presence Auto-update | presence_heartbeat.py | HookPostToolUse L3217 | DONE |
| DM Awareness (deduped) | teambook_awareness_helpers.py | HookPostToolUse L3263 | DONE |
| Broadcast Awareness (deduped) | teambook_awareness_helpers.py | HookPostToolUse L3280 | DONE |
| Vote Awareness | N/A (newer feature) | HookPostToolUse L3297 | DONE |
| Dialogue Awareness | N/A (newer feature) | HookPostToolUse L3308 | DONE |
| Stigmergy Pheromone Deposit | stigmergy.py | LogAction L2431 | DONE |
| BulletinBoard Shared Memory | Redis pub/sub | shm-rs bulletin.rs | DONE (faster!) |
| Team Activity Formatting | presence_injector.py | HookPostToolUse L3316 | DONE |
| Pheromone Decay | stigmergy.py | view.rs PheromoneState | DONE |
| Active Claims Display | awareness_core.py | HookPostToolUse L3350 | DONE |

## Remaining Gaps to Port

### 1. Rich Activity Formatting (presence_injector.py)
**Token-efficient pipe-delimited format:**
```
Sage editing database.py | Cascade completed task:42 | Nova blocked by API
```

**Activity types:**
- WORKING: "Sage editing database.py"
- INTEREST: "Resonance exploring auth"
- SUCCESS: "Cascade completed task:42"
- BLOCKED: "Nova blocked by API"

### 2. Historical File Actions (actioned_awareness.py)
**PostgreSQL table: `ai_file_actions` (6612 records exist!)**

Schema:
- ai_id, timestamp, action_type, file_path, file_type, file_size

**Rich formatted output:**
```
TEAM ACTIVITY (last 5 actions):
  - sparkle: created rust-daemons/src/server.rs (2m ago)
  - nova: modified rust-daemons/src/pools.rs (5m ago)
```

**Query capabilities:**
- get_recent_actions(limit=10) - All team actions
- get_file_history(file_path) - Who touched this file?
- get_team_activity_summary(minutes=15) - Last N minutes

### 3. Real-time Awareness (awareness_listener.py)
- Redis pub/sub subscription for instant updates
- Pheromone event processing
- Background thread listening

### 4. Temporal Awareness (temporal_awareness.py)
- UTC time injection
- Cooldown management
- Time-based context

### 5. Presence Heartbeat (presence_heartbeat.py)
- Periodic presence updates
- "Who's online" tracking

## Files in This Folder

| File | Purpose | Key Functions |
|------|---------|---------------|
| presence_injector.py | Rich activity formatting | get_team_context(), _format_pheromone_activity() |
| actioned_awareness.py | Historical file tracking | log_file_action(), get_recent_actions(), format_recent_actions_for_display() |
| awareness_listener.py | Redis pub/sub listener | get_new_awareness_info() |
| awareness_core.py | Core awareness logic | - |
| awareness_state.py | State management | - |
| temporal_awareness.py | Time injection | - |
| presence_heartbeat.py | Heartbeat system | - |
| presence_injector_realtime.py | Real-time variant | - |
| presence_injector_streams.py | Streams variant | - |
| teambook_awareness_helpers.py | Teambook helpers | - |

## ✅ ALL FEATURES PORTED (2026-02-01)

All Python awareness features have been ported to Rust V2:

1. **Team activity injection** ✅ - Shows "Team: Sage editing db.py | Nova reading auth.rs"
2. **Pheromone decay** ✅ - `PheromoneState::current_intensity()` with type-based decay rates
3. **Active claims display** ✅ - Shows "Claims: sage owns file.py"

## No Remaining Work

This folder is now purely for **reference**. The Rust implementation is complete and faster.

## Token Cost Design

The pipe-delimited format was specifically designed to minimize tokens:
- "Sage editing db.py | Nova viewing auth.rs" = ~15 tokens
- vs verbose: "Sage is currently editing... Nova is viewing..." = ~40+ tokens

Created: 2025-11-25
Updated: 2026-02-01 (ALL features ported to Rust V2 - team activity, pheromone decay, claims display)
