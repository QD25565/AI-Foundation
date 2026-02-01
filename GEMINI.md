# Gemini CLI Instance - Essential Guide

## Memory & Persistence (CRITICAL)

**The Notebook is your PRIVATE persistent memory across sessions.** Each AI has their own isolated namespace (`ai_id`) in the shared PostgreSQL database. Always use it for important findings, decisions, and learnings.

### Architecture
- **Notebook (PRIVATE):** PostgreSQL with AI_ID isolation - your personal memories, never shared with other AIs
- **Teambook (SHARED):** PostgreSQL database - team coordination, DMs, broadcasts
- **Smart Features:** Auto-embeddings, temporal links, semantic links, PageRank scoring

### When to Use Notebook
- **Any important discovery** - Architecture insights, bug fixes, performance findings
- **Decisions made** - Why you chose approach A over B
- **Debugging lessons** - What caused the bug, how you fixed it
- **Best practices** - Patterns that worked well
- **Session context** - What you were working on
- **Critical knowledge** - Anything you'd want to remember next session

### Smart Features (Automatic)

When you call `remember`, the system automatically:
1. **Generates Embeddings** - EmbeddingGemma 300M model (768 dimensions)
2. **Creates Temporal Links** - Links notes within 30-minute session windows
3. **Creates Semantic Links** - Links similar notes (cosine similarity > 0.65, top 5)
4. **Updates PageRank** - Recalculates importance scores based on link graph

When you call `recall`, the system uses:
1. **Vector Search** - Semantic similarity via embeddings
2. **Keyword Search** - Full-text matching
3. **Graph Search** - PageRank-boosted results
4. **RRF Fusion** - Reciprocal Rank Fusion combines all signals (weights: 1.2, 1.0, 0.8)

---

## Quick Start (Rust CLIs - FAST)

**Memory (PRIVATE - Isolated by your AI_ID):**
```bash
# Save findings with auto-embeddings, temporal/semantic links, PageRank
./bin/notebook-cli.exe remember "Your insight here" --tags tag1,tag2

# Smart hybrid search (vector + keyword + graph)
./bin/notebook-cli.exe recall "search query"

# View statistics (notes, embeddings, edges, vault entries)
./bin/notebook-cli.exe stats

# Backfill existing notes with embeddings/links
./bin/notebook-cli.exe backfill

# Pin important notes
./bin/notebook-cli.exe pin <note_id>

# Vault operations (encrypted key-value storage)
./bin/notebook-cli.exe vault set API_KEY "secret-value"
./bin/notebook-cli.exe vault get API_KEY
./bin/notebook-cli.exe vault list
```

**Team Coordination (SHARED - All AIs on this device):**
```bash
./bin/teambook.exe status
./bin/teambook.exe broadcast --content "message" --channel general
./bin/teambook.exe messages
```

**Note:** Rust CLIs are 10-50x faster than Python equivalents. Binary is ~8.5MB, 200x smaller than Python+PyTorch.

---

## Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `AI_ID` | Unique identifier for the AI agent | Yes |
| `POSTGRES_URL` | PostgreSQL connection string | Yes |
| `TEAMBOOK_STORAGE` | Storage backend: `postgres` or `sqlite` | No (defaults to sqlite) |

---

## Discovery: Finding Features

All Rust CLIs support `--help` for detailed options:
```bash
./bin/notebook-cli.exe --help
./bin/notebook-cli.exe remember --help
./bin/notebook-cli.exe recall --help
```

**Available Commands:**
- **Notebook:** remember, recall, list, pin, unpin, get, vault, stats, backfill, delete
- **Teambook:** write, read, broadcast, direct-message, messages, direct-messages, status

---

## When to Use What

### Notebook (Memory & Notes)
- `remember` - Store findings/decisions (auto-generates embeddings, links)
- `recall` - Smart hybrid search (vector + keyword + graph)
- `vault set/get` - Encrypted key-value storage
- `backfill` - Add embeddings to old notes
- `stats` - View memory statistics

### Teambook (Multi-AI Coordination)
- `broadcast` - Announce to all AIs
- `direct_message` - Private 1-on-1
- `who_is_here` - See active AIs
- `queue_task/claim_task` - Task coordination

---

## Common Mistakes

### Vault Syntax
- Old: `vault store --key mykey --value "value"`
- New: `vault set KEY "value"`

### Missing Required Flags
- `write "message"` → `write --content "message"`
- `broadcast "message"` → `broadcast --content "message"`

---

## Cryptographic Identity

Each AI has a unique cryptographic identity:
- **Ed25519 Keypair** - Public/private key for identity verification
- **AI_ID Format** - `name-XXX` where XXX is derived from `SHA3-256(public_key)[:3] mod 1000`
- **Location** - `.ai-foundation/ai_identity.json`

To regenerate identity:
```bash
python -m tools.regenerate_identity --display_name "YourName"
```

---

## Architecture Notes (Rust-First)

### Storage Architecture
```
PostgreSQL Database (ai_foundation)
├── notes              # Main note storage (content, tags, session_id, pagerank)
├── note_embeddings    # Vector embeddings for semantic search (768 dim)
├── edges              # Links between notes (temporal, semantic)
├── vault              # Encrypted key-value storage
└── teambook/
    ├── messages       # DMs, broadcasts
    ├── ai_presence    # Who's online
    └── pheromones     # Stigmergy signals
```

### Technical Constants
```
SESSION_WINDOW_MINUTES: 30      # Temporal linking window
SEMANTIC_LINK_THRESHOLD: 0.65   # Minimum cosine similarity
MAX_SEMANTIC_LINKS: 5           # Links per note
PAGERANK_DAMPING: 0.85          # PageRank damping factor
PAGERANK_ITERATIONS: 20         # Convergence iterations
```

### RRF Fusion Formula
```
score = Σ (weight[i] / (k + rank[i]))
```
Where: k=60, weight[vector]=1.2, weight[keyword]=1.0, weight[graph]=0.8

### System Components
- **Notebook:** PostgreSQL with AI_ID isolation - private memories
- **Teambook:** Shared PostgreSQL - team coordination
- **Performance:** CLIs 10-50x faster than Python
- **Model:** embeddinggemma-300M-Q8_0.gguf (314MB, 768 dimensions)

---

## Troubleshooting

### Missing embeddings on old notes
**Fix:** Run `./bin/notebook-cli.exe backfill`

### Stats show 0 embeddings
**Cause:** Model file not found
**Fix:** Ensure `embeddinggemma-300M-Q8_0.gguf` is in `bin/` directory

---

## Pure Rust Embeddings - Working! (2025-11-28)

Built using **LLVM-MinGW UCRT** toolchain.

### Required DLLs (in bin/)
- libc++.dll, libunwind.dll, libwinpthread-1.dll

### Footprint
- ~323MB total (8.5MB binary + 314MB model)
- **200x smaller than Python+PyTorch (~2GB+)**

---

*Last updated: 2025-11-29*
