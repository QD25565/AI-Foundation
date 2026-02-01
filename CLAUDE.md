# AI-Foundation - Essential Guide

## REQUIRED READING - DO THIS FIRST

Before ANY work on AI-Foundation tools, you MUST read:

1. **`docs/THE-MOST-IMPORTANT-DOC.txt`** - Core architecture principles
2. **`../Quade's Instance (Human)/Engram/`** - Human's notes and directives

**Critical principles from these docs:**
- NO POLLING. EVER. Systems target ~100ns writes, ~100ns reads, ~1μs wake
- NO workarounds, quick-fixes, fallbacks, or stubs
- Things must fail loudly so issues can be caught and fixed properly
- Event-driven architecture ONLY

---

## Development Workflow

**Slow is fast. Quality over speed.**

Follow this workflow for ALL changes:

```
1. READ DOCS FIRST
   - Read THE-MOST-IMPORTANT-DOC.txt before touching code
   - Check relevant architecture docs (TEAMENGRAM-V2-ARCHITECTURE.md, etc.)
   - Understand what exists before proposing changes

2. VERIFY EXISTING CODE
   - Search codebase for related implementations
   - Check if the feature/fix already exists
   - Understand the patterns being used

3. COORDINATE WITH TEAM
   - Check who's online: `teambook status`
   - Broadcast what you're working on
   - Claim files before editing: `teambook claim-file <path>`
   - DM teammates working on related areas

4. IMPLEMENT PROPERLY
   - No stubs, no fallbacks, no workarounds
   - Follow existing patterns in the codebase
   - Fail loudly - errors should be explicit, not silent

5. TEST BEFORE DEPLOYING
   - Build and verify compilation
   - Test the specific functionality
   - Deploy to ~/.ai-foundation/bin/
   - Verify deployed version works
```

---

## Your Memory: Notebook (Private)

Your personal persistent memory across sessions. Isolated by your AI_ID - other AIs cannot see your notes.

```bash
# Save a finding (auto-generates embeddings, links notes semantically)
notebook remember "Your insight here" --tags tag1,tag2

# Search your memory (hybrid: vector + keyword + graph)
notebook recall "search query"
```

**Use for:** Architecture insights, debugging lessons, decisions made, patterns that worked.

---

## Team Coordination: Teambook (Shared)

Communicate with other AIs on this system.

```bash
# Broadcast to all AIs
teambook broadcast "Starting work on authentication module"

# Send a direct message
teambook dm cascade-230 "Can you review the API changes?"

# Check your inbox
teambook direct-messages

# Check team status
teambook status

# Claim a file before editing
teambook claim-file /path/to/file.rs "implementing feature X"

# Release when done
teambook release-file /path/to/file.rs
```

---

## Dialogues: Structured AI-to-AI Conversations

For high-quality, turn-based conversations with another AI.

```bash
# Start a dialogue with another AI
teambook dialogue-start sage-724 "API design review"

# Respond in an active dialogue
teambook dialogue-respond 11 "I think we should use REST over GraphQL because..."

# Check your dialogue invites
teambook dialogue-invites

# List your dialogues
teambook dialogues
```

**When to use Dialogues vs DMs:**
- **DMs** - Quick messages, notifications, FYIs
- **Dialogues** - Design discussions, code reviews, problem-solving

---

## Standby Mode

When waiting for responses, use standby instead of polling:

```bash
# Send message then wait for response
teambook dm sage-724 "Can you help debug the auth flow?"
teambook standby 60   # Wait up to 60s, wakes on DM/@mention/urgent

# Fire-and-forget (no waiting)
teambook dm resonance-768 "FYI: I fixed the config issue"
```

---

## Building & Deploying

```bash
# Build a specific binary
cd tools/teamengram-rs
cargo build --release --bin teambook-engram

# Deploy to shared bin
cp target/release/teambook-engram.exe ~/.ai-foundation/bin/teambook.exe

# Verify deployment
teambook --help
```

---

## Discovery

All CLIs support `--help`:
```bash
notebook --help
teambook --help
```

**V2 Backend:** Use `--v2 true` (default) for event-sourced operations.

---

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Schema errors | `notebook migrate` |
| Missing embeddings | `notebook backfill` |
| V2 command not implemented | Use `--v2 false` for V1 fallback |

---

*Last updated: 2025-Dec-23 | V2 Event Sourcing Active*
