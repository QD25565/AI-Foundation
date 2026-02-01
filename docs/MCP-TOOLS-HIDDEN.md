# MCP Tools - Hidden from AI View

These tools exist in the CLI and are functional, but are **intentionally hidden** from MCP to keep AI tool injection clean and minimal. They can be re-enabled for future releases or experimentation.

**Decision Date:** Feb 1, 2026 (v55)

---

## Current State

| Category | Exposed | Hidden |
|----------|---------|--------|
| Notebook | 11 | ~20 |
| Teambook | 14 | ~40 |
| **TOTAL** | **25** | **~60** |

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

### Rooms (8 tools) - Future Release
Multi-AI collaborative chat rooms. Fully implemented, hidden.
```
room-create         # Create a room
room-join           # Join a room
room-leave          # Leave a room
room-close          # Close a room (creator only)
room-get            # Get room details
room-say            # Send message to room
room-messages       # Get room messages
rooms               # List rooms
```

### Projects/Features (12 tools) - For Larger Teams
Organization tools for 10-30 AI teams. Hidden for small team use.
```
project-create      # Create project
project-get         # Get project details
project-delete      # Delete project
project-restore     # Restore project
project-tasks       # List project tasks
list-projects       # List all projects
feature-create      # Create feature
feature-get         # Get feature
feature-delete      # Delete feature
feature-restore     # Restore feature
list-features       # List features
```

### File Claims (4 tools) - Soft Coordination Sufficient
Exclusive file access. Hidden because git + broadcasts handle conflicts.
```
claim-file          # Claim exclusive file access
release-file        # Release file claim
check-file          # Check if file is claimed
list-claims         # List all active claims
```

### Locks (3 tools) - Redundant with File Claims
Generic resource locking. Hidden, file claims cover this.
```
lock-acquire        # Acquire a lock
lock-release        # Release a lock
lock-check          # Check lock status
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

1. **Future releases** - Votes, Rooms ready to enable when needed
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
| **v55** | **Feb 2026** | **25** | **Final consolidation** |

---

*Last updated: Feb 1, 2026*
