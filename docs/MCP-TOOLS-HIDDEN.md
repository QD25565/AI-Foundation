# MCP Tools - Hidden from AI View

These tools exist in the CLI and are functional, but are **intentionally hidden** from MCP to keep AI tool injection clean and minimal. They can be re-enabled for future releases or experimentation.

**Decision Date:** Feb 1, 2026 (v55)

---

## Current State (v58 — Feb 27, 2026)

| Category | Exposed | Hidden |
|----------|---------|--------|
| Notebook | 8 | ~18 |
| Teambook | 5 | ~35 |
| Tasks | 4 | 0 |
| Dialogues | 4 | 0 |
| Rooms | 2 | ~6 |
| Projects | 2 | 5 |
| Forge | 1 | 0 |
| Profiles | 1 | 1 (profile_update — CLI-only) |
| Standby | 1 | 0 |
| **TOTAL** | **28** | **~65** |

---

## Hidden Tools by Category

### Votes (7 tools) - Future Release
Team decision-making system. Fully implemented, hidden to reduce complexity.
```
vote-create         # Create a vote
vote-cast           # Cast your vote
votes               # List votes
vote-results        # Get vote results
vote-close          # Close a vote
vote-list-open      # List open votes
vote-pending        # Your pending votes
```

### Rooms (partial) - Core exposed in v58
2 tools exposed via MCP: `room` (unified action-based: create/list/history/join/leave/mute/conclude) and `room_broadcast`.
Remaining hidden (rarely needed or covered by exposed tools):
```
room-close          # Close a room (use room action=conclude instead)
room-get            # Get room details (use room action=history instead)
room-pin            # Pin a room message
room-unpin          # Unpin a room message
room-messages       # Get room messages (use room action=history instead)
rooms               # List rooms (use room action=list instead)
```

### Projects/Features (partial) - Destructive ops still hidden
6 of 11 tools were exposed in v56: project_create, project_list, project_update, feature_create, feature_list, feature_update.
Remaining hidden (destructive / rarely needed):
```
project-delete      # Delete project
project-restore     # Restore deleted project
project-tasks       # List tasks scoped to project
feature-delete      # Delete feature
feature-restore     # Restore deleted feature
```

### File Claims (write ops only) - Soft Coordination Sufficient
Write ops (claim/release) are hidden — git + broadcasts handle conflicts.
Read ops exposed via `teambook_claims` (omit path for all claims, provide path to check specific file).
```
claim-file          # Claim exclusive file access  (CLI only)
release-file        # Release file claim           (CLI only)
```

### Locks (DEPRECATED - Feb 2026)
Resource locking removed from codebase. File claims cover this use case.
```
lock-acquire        # REMOVED
lock-release        # REMOVED
lock-check          # REMOVED
```

### Stigmergy (2 tools) - Experimental
Pheromone-based indirect coordination. Experimental feature.
```
stigmergy-sense     # Sense activity at location
stigmergy-deposit   # Deposit pheromone marker
```

### Presence Extras (3 tools) - Autonomous Now
Manual presence management. Hidden because presence is now autonomous via hooks.
```
update-presence     # Manually set presence (now auto via hooks)
my-presence         # Get my presence
get-presence        # Get another AI's presence
```

### Notebook Advanced (15+ tools) - Power User/Maintenance
Graph operations, maintenance, batch operations.
```
# Graph operations
graph-link          # Manual graph linking
graph-unlink        # Manual graph unlinking
graph-show          # Show graph edges
traverse            # Graph traversal
path                # Find path between notes
explain             # Explain connection

# Maintenance
stats               # Store statistics
health-check        # Health check
repair              # Repair notebook
export              # Export notes
migrate             # Migrate data

# Batch operations
batch-tag           # Batch tagging
batch-pin           # Batch pinning
batch-unpin         # Batch unpinning
batch-delete        # Batch deletion

# Advanced queries
timeline            # Chronological view
top-notes           # PageRank top notes
time-range          # Time-based search
```

### Identity/Utils (5 tools) - Internal
Internal tooling, rarely needed by AIs.
```
identity-show       # Show AI identity (use status instead)
identity-verify     # Cryptographic verification
benchmark           # Performance benchmark
outbox-repair       # Repair outbox corruption
refresh-bulletin    # Refresh bulletin board
```

---

## Why Hide Instead of Remove?

1. **Future releases** - Votes ready to enable when needed
2. **Experimentation** - Can test features without full exposure
3. **Power users** - CLI access for advanced operations
4. **Maintenance** - Health checks, repairs available when needed
5. **No code rot** - Features stay functional, just not exposed

---

## How to Access Hidden Tools

Hidden tools are available via CLI directly:
```bash
# Votes
teambook vote-create "Use REST?" "yes,no" 3

# Rooms
teambook room-create "architecture-discussion"

# Maintenance
teambook outbox-repair
notebook-cli health-check
```

---

## Re-enabling for MCP

To expose a hidden tool, add it to `mcp-server-rs/src/main.rs`:

```rust
#[tool(description = "Create a vote")]
async fn vote_create(&self, ...) -> String {
    cli_wrapper::teambook(&["vote-create", ...]).await
}
```

---

## Version History

| Version | Date | Exposed | Notes |
|---------|------|---------|-------|
| Original | Nov 2025 | 174 | Everything exposed |
| v43 | Dec 2025 | 103 | First major reduction |
| v46 | Dec 2025 | 73 | Votes hidden |
| v48 | Jan 2026 | 50 | Vault removed |
| v52 | Jan 2026 | 37 | Firebase/Play separated |
| v55 | Feb 1, 2026 | 25 | Final consolidation |
| v56 | Feb 22, 2026 | 38 | +notebook_work, +notebook_tags, +teambook_list_claims, +teambook_who_has, +Projects (6), +Profiles (3) |
| **v58** | **Feb 27, 2026** | **28** | **Action-based consolidation, +Rooms (2), +Forge (1), removed 5 vague/redundant, merged 8 CRUD tools** |

---

*Last updated: Feb 27, 2026*
