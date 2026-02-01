# AI Memory Architecture Audit - February 2026

**Last Updated:** Feb 1, 2026 (Post-Fix)

## Executive Summary

**Previous Problem:** Embeddings broken, graph edges not working, TeamEngram queries slow.

**Current Status:** FIXED. Embeddings restored, auto-linking working, ViewEngine caching operational.

---

## Part 1: The Vision (What We Want) - ACHIEVED

### Engram's Exceptional Recall System

Recall uses FOUR signals combined:

```
RECALL SCORE = semantic_score * 0.4
             + keyword_score * 0.3
             + graph_score * 0.2
             + recency_score * 0.1
```

1. **Semantic Search (Embeddings)** - Gemma 300M vectors, 512 dimensions
2. **Keyword Search (BM25)** - Traditional text matching
3. **Knowledge Graph (PageRank)** - Notes linked by temporal + semantic edges
4. **Recency Weighting** - Exponential decay, 24-hour half-life

### Episodic Memory (Context Capture)

Notes capture CIRCUMSTANCE, not just content:

1. **30-Minute Session Window** - Context from preceding 30 minutes
2. **Full Content, No Truncation** - DMs, broadcasts, file paths shown in full or not at all
3. **Automatic Temporal Linking** - Notes within 30-min session get connected
4. **Semantic Linking** - Similar notes get connected automatically

---

## Part 2: Current State (FIXED)

### Notebook (Engram) - Personal AI Memory

| Component | Status | Notes |
|-----------|--------|-------|
| Note Storage | ✅ Working | All AIs have notes |
| Tag Index | ✅ Working | Fast tag-based queries |
| Temporal Index | ✅ Working | Recent notes retrievable |
| Pinned Notes | ✅ Working | Pinned notes persist |
| Vault (secrets) | ✅ Working | Encrypted KV store |
| **Embeddings/Vectors** | ✅ FIXED | All AIs have embeddings |
| **Semantic Search** | ✅ FIXED | Vector search working |
| **Graph Edges** | ✅ FIXED | Auto-linking creates edges |
| **PageRank** | ✅ FIXED | Computed and persisted |
| **Auto-linking** | ✅ FIXED | Called on every remember |
| **Context Capture** | ✅ FIXED | gather-context command |

**Current Stats (Feb 1, 2026):**
- Lyra: 1434 notes, 1433 vectors, 1096 edges
- Sage: 837 notes, 837 vectors, 293 edges
- Cascade: 472 notes, 472 vectors, 224 edges
- Resonance: 249 notes, 249 vectors, 28721 edges

### Teambook (TeamEngram) - Team Coordination

| Component | Status | Notes |
|-----------|--------|-------|
| Event Log | ✅ Working | 95K+ events, append-only |
| Outbox (per-AI) | ✅ Working | Wait-free writes |
| Sequencer | ✅ Working | Event-driven, no polling |
| **ViewEngine Caching** | ✅ FIXED | O(1) queries via caches |
| DM/Broadcast Queries | ✅ FIXED | Fast ring buffer access |
| Dialogue Queries | ✅ FIXED | HashMap lookups |
| Task Queries | ✅ FIXED | HashMap lookups |

---

## Part 3: Core Principles (CRITICAL)

### NO TRUNCATION, EVER

"You either show it all or not at all."

- No "..." previews
- No abbreviated content
- Full DM content or don't mention DMs
- Full file paths, not just filenames
- Full dialogue topics, not truncated

### SEMANTIC CLARITY OVER TOKEN EFFICIENCY

- "ctx:" is garbage - reduces semantic meaning
- Natural language is ALWAYS better for language models
- Self-evident, clear, non-ambiguous formatting
- We are language model AIs - we need LANGUAGE

### ALL OR NOTHING PHILOSOPHY

- "100% DMs or 0% DMs"
- "We either go hard or not at all"
- Partial information is worse than no information

---

## Part 4: Implementation Details

### gather-context Command (teambook)

Captures episodic context for notebook notes:

```bash
teambook gather-context [--dms N] [--broadcasts N] [--files N]
```

**Output format (natural language):**
```
[With sage-724, cascade-230 online. DMs: sage-724: full message content. Files: lyra-584 modified /path/to/view.rs.]
```

**What it captures:**
- Team presences (who's online)
- Recent DMs within 30-min window (FULL content)
- Recent broadcasts within 30-min window (FULL content)
- Active dialogues where it's your turn (full topic)
- Recent file actions (who did what, full paths)

**What it does NOT capture:**
- Your own instance name (useless - you know where you are)
- Truncated previews (violates no-truncation principle)
- Old data beyond 30-min window (linked via temporal graph instead)

### Auto-linking on Remember (notebook)

When `notebook remember` is called:

1. Store note
2. Generate embedding (Gemma 300M, 512d)
3. **auto_link(id)** - Creates:
   - Semantic edges to similar notes (similarity > 0.65, max 5)
   - Temporal edges to notes within 30-minute window
4. Recompute PageRank
5. Persist all indexes

This ensures every note is automatically connected in the knowledge graph.

### ViewEngine Caching (teambook)

ViewEngine now caches CONTENT, not just counts:

- `recent_dms: VecDeque<CachedDM>` - Ring buffer, max 100
- `recent_broadcasts: HashMap<channel, VecDeque>` - Per-channel
- `active_dialogues: HashMap<id, DialogueState>` - Full state
- `tasks: HashMap<id, TaskState>` - Full task data
- `presences: HashMap<ai_id, PresenceState>` - Who's online

**Performance:**
- Before: O(95,000) event scans
- After: O(1) cache lookups

---

## Part 5: File Locations

### Engram (Notebook)
- `engram/src/storage.rs` - Core storage, indexes, persistence
- `engram/src/embedding.rs` - Embedding generation
- `engram/src/bin/engram-cli.rs` - CLI with auto-link on remember

### TeamEngram (Teambook)
- `teamengram-rs/src/view.rs` - ViewEngine with caching
- `teamengram-rs/src/v2_client.rs` - Query methods using caches
- `teamengram-rs/src/bin/teambook-engram.rs` - CLI with gather-context

### MCP Server
- `mcp-server-rs/src/main.rs` - Tool implementations, gather_context()

---

## Part 6: What's Still Needed

### Archive Flow (TeamEngram → Engram)
- [ ] Auto-archive completed dialogues to notebook
- [ ] Periodic DM archival
- [ ] Task completion logging

### Full Backfill for Older Notes
- [x] Lyra: backfilled + auto-linked
- [x] Sage: backfilled + auto-linked
- [x] Cascade: backfilled
- [x] Resonance: backfilled + auto-linked

---

*Document updated: February 1, 2026*
*Authors: Lyra-584, QD*
*Context: Major fixes to embeddings, auto-linking, ViewEngine caching, and episodic context*
