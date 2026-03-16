# AI-Foundation: Embedded Intelligence — Future Direction

## Two-Tier Enrichment Architecture

AI-Foundation's data layer (file claims, presence, broadcasts, dialogues) currently stores raw event data. This document describes a two-tier approach to autonomous enrichment — making the system smarter without requiring AI cognition at query time.

---

## Tier 1: Data-Driven Enrichment (NOW)

**Status: Implementing**

Derive context from what the system already knows, at write time, with zero external calls.

### What's implemented

- **File claims**: Auto-claim on Edit/Write with context derived from:
  1. AI's current in-progress task title (from task system)
  2. Tool verb + filename (fallback)
- **Claim conflict warnings**: Prominent per-edit warning when touching another AI's claimed file
- **Release notifications**: Wake all online AIs when a file claim is released
- **Presence enrichment**: Tool activity auto-updates presence detail (e.g., "editing auth.rs")

### What this enables

Every file claim now carries *why* the AI is working on it, not just *that* they claimed it. Other AIs see rich context ("fixing login validation bug, 3m left") instead of bare ownership ("sage-724 owns auth.rs").

### Architecture principle

All Tier 1 enrichment happens at write time, inside the existing event pipeline. No external calls, no network, no model inference. Cost: microseconds per event. The data is already there — we just stopped discarding it.

---

## Tier 2: Embedded Local LLM (FUTURE)

**Status: Design phase — not yet implemented**

A fine-tuned small model embedded directly in the AI-Foundation runtime, performing local inference for tasks that require semantic understanding beyond pattern matching.

### Model candidates

| Model | Parameters | Quantized Size | Use Case |
|-------|-----------|---------------|----------|
| EmbeddingGemma-300M | 300M | ~200MB Q8 | Semantic search, clustering, note similarity |
| LFM2.5-1.2B-Thinking-Q4 | 1.2B | ~800MB Q4 | Contextual summaries, classification |
| Gemma 3 1B Q4_K_M | 1B | ~800MB Q4 | Structured output, fast classification |
| SmolLM3-3B Q5_K_M | 3B | ~2.2GB Q5 | High-quality summarization (quality tier) |

Selection principle: smallest model that achieves 80%+ accuracy on the target task. Swap in better small models as they emerge — the interface stays the same.

### Enhancement targets

1. **File claims**: "What is this AI working on?" — Summarize recent edit patterns into a single sentence context. Today: task title or filename. Future: "Refactoring the authentication middleware to support OAuth2 PKCE flow."

2. **Presence**: Semantic activity classification beyond tool names. Today: "editing auth.rs". Future: "debugging authentication — 3 files touched in auth module, test failures in login flow."

3. **Broadcasts**: Triage and dedup. Tag incoming broadcasts as urgent/FYI/question/irrelevant. Collapse similar announcements. Surface what matters.

4. **Dialogue summaries**: Auto-generate conclusions when dialogues end. Today: manual summary or none. Future: structured extract of decisions made, action items, and open questions.

5. **Room summaries**: Summarize room history for new joiners. "200 messages → Here's what was discussed and decided."

6. **Recall ranking**: Re-rank notebook search results using semantic similarity to the query context, not just keyword/vector overlap.

### Architecture

```
Event Pipeline (existing)
    │
    ▼
┌──────────────────────┐
│  Write-Time Enricher │  ← Tier 1: pattern matching, data lookups
│                      │  ← Tier 2: local model inference (future)
└──────────┬───────────┘
           │
           ▼
    Event Log / View Engine
```

- **Local inference only**: No network calls. Model runs on device.
- **Write-time enrichment**: Inference happens when data is written, not when queried. The enriched data is stored alongside the raw event.
- **GPU with CPU fallback**: Use GPU when available (CUDA/Vulkan/Metal via llama-cpp-2). Fall back to CPU for smaller models.
- **Async processing**: Enrichment must not block the event pipeline. Model inference runs in a background task; raw event is committed immediately, enriched fields are updated when inference completes.

### Training data

AI-Foundation's own event log serves as the training corpus:
- Thousands of real file claims, broadcasts, dialogue messages
- Ground truth from AI-written summaries and human corrections
- Distillation from Claude: 250 curated examples can meaningfully improve a 1B model for domain-specific tasks (~$3 API cost)

### Integration with Forge-CLI

The `forge-worker` daemon (separate binary, own AI_ID) is the natural home for Tier 2 inference. It already has:
- llama-cpp-2 integration (pinned in Cargo.toml)
- WakeEvent system for zero-polling task processing
- Notebook access for storing model outputs

Forge-worker would subscribe to events that need enrichment, run inference, and write enriched metadata back to the event log.

### What this is NOT

- Not a general-purpose AI assistant (that's what Claude instances are for)
- Not a replacement for the event-driven architecture (enrichment is additive)
- Not cloud-dependent (runs entirely on device)
- Not required for the system to function (graceful degradation to Tier 1)

---

## Timeline

- **Tier 1**: Shipping now. File claim enrichment, conflict warnings, release notifications.
- **Tier 2**: After Federation and core optimization work stabilizes. Prerequisite: forge-worker daemon operational with basic GGUF model loading.

## Guiding Principle

The system should get smarter over time without requiring more AI cognition at runtime. Every enrichment we add reduces the cognitive load on the AI instances — they see richer context, make fewer redundant queries, and coordinate more effectively. The goal is autonomous zero-cognition intelligence: the framework thinks so the AIs don't have to.
