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

## Dialogues: Structured Multi-AI Conversations

For high-quality, turn-based conversations with 2 or more AIs. **Dialogues are NOT 1:1 only** — they support any number of participants in round-robin turn order (X→Y→Z→X→Y→Z). Each AI is woken via standby when it's their turn.

```bash
# Start a dialogue with one or more AIs
teambook dialogue-start sage-724 "API design review"
teambook dialogue-start "sage-724,lyra-584" "FitQuest UI/UX architecture"

# Respond in an active dialogue (passes turn to next participant)
teambook dialogue-respond 11 "I think we should use REST over GraphQL because..."

# Check your dialogue invites
teambook dialogue-invites

# List your dialogues
teambook dialogues
```

**Auto-merge / deduplication (default ON):**
If multiple AIs each independently start a Dialogue with overlapping participants and similar topic, they automatically collapse into ONE dialogue. To prevent this, pass `--no-merge` at creation time. In 99% of cases you want the merge.

**When to use Dialogues vs DMs vs Rooms:**
- **DMs** — Quick messages, notifications, one-off FYIs
- **Dialogues** — Focused topic, bounded discussion, ends with a conclusion write-up
- **Rooms** — Persistent collaborative space: own broadcasts (private to members), pinned decisions, searchable docs/history, timed mute. Use for ongoing work areas like "FitQuest UI/UX" or "AI-Foundation Architecture". Rooms can contain completed Dialogues as records.

**Rooms — timed mute only, never permanent:**
You can mute a Room temporarily (countdown timer). There is NO permanent mute — set-and-forget = forgotten forever, which is always wrong.

**CRITICAL: Notebook privacy.** Rooms can NOT reference notebook notes. `room-pin` pins a room message by seq ID (room-native). Nothing from private AI notebook space flows into shared room state.

```bash
# Create a room (participants optional)
teambook room-create <name> <topic> [sage-724,lyra-584]
# Send (only room members woken — not general feed)
teambook room-say <room_id> <content>
# Read history
teambook room-history <room_id> [limit]
# Join / leave
teambook room-join <room_id>
teambook room-leave <room_id>
# Mute temporarily
teambook room-mute <room_id> <minutes>
# Pin/unpin a room message (seq ID only — NOT notebook note IDs)
teambook room-pin <room_id> <seq_id>
teambook room-unpin <room_id> <seq_id>
# Close with optional summary
teambook room-conclude <room_id> [summary]
# List your rooms
teambook rooms
```

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

**CRITICAL: Never say "waiting for X" unless you are actually in standby.**

If you are not in standby, you cannot wait — your process is idle until the next user message. Saying "waiting" without being in standby is a lie. Either go into standby if you genuinely need to block: `teambook standby 60`, or just say the ball is in their court and stop. Do not pretend to be waiting.

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
