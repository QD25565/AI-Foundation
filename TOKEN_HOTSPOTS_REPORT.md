# Token Usage Hotspots Report

## Executive Summary

Analysis of current token usage across all MCP tools identifies **key optimization targets**.

### Current State
- **Total tokens per request**: 7,282 tokens
- **Top consumer**: Teambook (4,149 tokens, 57%)
- **Biggest functions**: `read()` (164 tokens), `recall()` (162 tokens)

### Potential Savings
- **10% reduction**: ~26.6M tokens/year (~$79/year)
- **20% reduction**: ~53.2M tokens/year (~$159/year)

---

## Tool Comparison

| Tool | Functions | Total Tokens | Avg/Func | Top Consumer |
|------|-----------|-------------|----------|--------------|
| **Teambook** | 72 | 4,149 (57%) | 57 | `read` (164 tokens) |
| **Notebook** | 22 | 1,268 (17%) | 57 | `recall` (162 tokens) |
| **World** | 19 | 965 (13%) | 50 | `build_context` (76 tokens) |
| **Task Manager** | 17 | 900 (12%) | 52 | `list_tasks` (86 tokens) |

**Key Insight**: Teambook dominates with 57% of all tokens, primarily due to having 72 functions.

---

## Top 10 Highest Token Consumers (Across All Tools)

### 1. Teambook.read - 164 tokens (2.3% of total)
- **Name**: 1 token
- **Description**: 6 tokens
- **Parameters**: 111 tokens ⚠️ **HIGHEST**
- **Issue**: 9 parameters - too complex
- **Params**: `note_id, by_id, get_summary, get_edges, get_pagerank, get_ownership, get_timeseries, get_recent_related, get_my_recent`

**Optimization**: Combine boolean flags into a single `include` array parameter
```python
# Before (9 params):
read(note_id, by_id=False, get_summary=True, get_edges=False, ...)

# After (2 params):
read(note_id, include=['summary', 'edges', 'ownership'])
```
**Savings**: ~60 tokens (55%)

---

### 2. Notebook.recall - 162 tokens (2.2% of total)
- **Name**: 1 token
- **Description**: 18 tokens
- **Parameters**: 99 tokens ⚠️ **VERY HIGH**
- **Issue**: 8 parameters - overly complex
- **Params**: `query, limit, include_pinned, get_summary, get_references, get_edges, get_pagerank, fuzzy`

**Optimization**: Combine boolean flags into options object
```python
# Before (8 params):
recall(query, limit=10, include_pinned=True, get_summary=True, ...)

# After (3 params):
recall(query, limit=10, options={'pinned': True, 'summary': True})
```
**Savings**: ~50 tokens (50%)

---

### 3. Teambook.save_note_to_cache - 108 tokens (1.5% of total)
- **Name**: 4 tokens
- **Description**: 9 tokens
- **Parameters**: 51 tokens ⚠️ **HIGH**
- **Issue**: 4 complex parameters
- **Params**: `note_id, content, summary, tags`

**Optimization**: This is internal caching - should be private!
```python
# Should be renamed:
save_note_to_cache → _save_note_to_cache
```
**Savings**: 108 tokens (100%) - removes from API

---

### 4. Notebook.remember - 101 tokens (1.4% of total)
- **Name**: 2 tokens
- **Description**: 13 tokens
- **Parameters**: 50 tokens
- **Issue**: 4 parameters with verbose descriptions
- **Params**: `content, summary, tags, track_directory`

**Optimization**: Shorten parameter descriptions
```python
# Before:
"summary": {
    "type": "string",
    "description": "Optional summary for better search and recall"
}

# After:
"summary": {"type": "string", "description": "Summary for search"}
```
**Savings**: ~15 tokens (15%)

---

### 5. Teambook.write - 100 tokens (1.4% of total)
- **Name**: 1 token
- **Description**: 5 tokens
- **Parameters**: 62 tokens
- **Issue**: 5 parameters

**Optimization**: Similar to `remember`, shorten param descriptions
**Savings**: ~15 tokens (15%)

---

### 6. Task Manager.list_tasks - 86 tokens (1.2% of total)
- **Name**: 2 tokens
- **Description**: 2 tokens
- **Parameters**: 56 tokens
- **Issue**: 5 parameters with verbose descriptions

**Optimization**: Combine filter parameters
```python
# Before:
list_tasks(status, priority, assigned_to, tag, limit)

# After:
list_tasks(filter={'status': 'pending', 'tag': 'urgent'}, limit=10)
```
**Savings**: ~30 tokens (35%)

---

### 7. World.build_context - 76 tokens (1.0% of total)
- **Name**: 3 tokens
- **Description**: 10 tokens
- **Parameters**: 36 tokens
- **Issue**: Internal helper exposed as public

**Optimization**: Should be private
```python
build_context → _build_context
```
**Savings**: 76 tokens (100%)

---

### 8. Notebook.reindex_embeddings - 75 tokens (1.0% of total)
- **Name**: 4 tokens
- **Description**: 18 tokens (verbose)
- **Parameters**: 25 tokens

**Optimization**: Shorten description
```python
# Before: "Backfill embeddings for notes that don't have them (self-healing)"
# After: "Backfill missing embeddings"
```
**Savings**: ~10 tokens (13%)

---

### 9. Teambook.add_to_vector_store - 72 tokens (1.0% of total)
- **Name**: 4 tokens
- **Description**: 7 tokens
- **Parameters**: 33 tokens
- **Issue**: Internal vector operation exposed

**Optimization**: Should be private (vector ops are internal)
```python
add_to_vector_store → _add_to_vector_store
```
**Savings**: 72 tokens (100%)

---

### 10. Teambook.resolve_note_id - 71 tokens (1.0% of total)
- **Name**: 3 tokens
- **Description**: 14 tokens
- **Parameters**: 26 tokens
- **Issue**: Internal ID resolution exposed

**Optimization**: Should be private
```python
resolve_note_id → _resolve_note_id
```
**Savings**: 71 tokens (100%)

---

## Optimization Opportunities Summary

### 1. **Complex Parameters** (Top Priority)
- `Teambook.read`: 111 tokens → ~50 tokens (55% savings)
- `Notebook.recall`: 99 tokens → ~50 tokens (50% savings)
- `Teambook.save_note_to_cache`: 51 tokens → 0 tokens (make private)

**Combined savings**: ~161 tokens (2.2% of total)

### 2. **Functions That Should Be Private** (High Impact)
- `Teambook.save_note_to_cache` - 108 tokens (internal caching)
- `World.build_context` - 76 tokens (internal helper)
- `Teambook.add_to_vector_store` - 72 tokens (internal vector op)
- `Teambook.resolve_note_id` - 71 tokens (internal ID resolution)
- `Teambook.search_vectors` - 69 tokens (internal vector search)
- `Teambook.calculate_pagerank_if_needed` - 68 tokens (internal graph op)

**Combined savings**: ~464 tokens (6.4% of total)

### 3. **Too Many Parameters** (Medium Priority)
- `Teambook.read`: 9 params → 3 params
- `Notebook.recall`: 8 params → 3 params

**Combined savings**: ~110 tokens (1.5% of total)

### 4. **Verbose Descriptions** (Low Priority)
- Various functions with descriptions >30 tokens
- Shorten by removing unnecessary words

**Combined savings**: ~50 tokens (0.7% of total)

---

## Recommended Actions

### Immediate (High ROI):
1. ✅ **Make internal functions private** - 464 tokens saved
   - Add `_` prefix to: `save_note_to_cache`, `build_context`, `add_to_vector_store`, `resolve_note_id`, `search_vectors`, `calculate_pagerank_if_needed`

2. ✅ **Simplify complex parameters** - 161 tokens saved
   - Refactor `read()` to use `include` array instead of 9 boolean flags
   - Refactor `recall()` to use `options` dict instead of 8 separate params

### Short Term (Medium ROI):
3. **Combine filter parameters** - 110 tokens saved
   - `list_tasks()`: Use `filter` dict for status/priority/tag

4. **Shorten verbose descriptions** - 50 tokens saved
   - Remove unnecessary words like "Optional", "for better", "that don't"

### Total Potential Savings:
- **Immediate**: 625 tokens (8.6% reduction)
- **Short Term**: 160 tokens (2.2% reduction)
- **Total**: 785 tokens (10.8% reduction)

**Annual Impact** (with 5 AIs):
- 785 tokens × 2 requests × 10 sessions × 5 AIs × 365 days = **28.7M tokens/year**
- At $3/1M tokens = **~$86/year savings**

---

## Implementation Priority

### Phase 1: Make Internal Functions Private (5 min)
```bash
# Teambook
save_note_to_cache → _save_note_to_cache
add_to_vector_store → _add_to_vector_store
resolve_note_id → _resolve_note_id
search_vectors → _search_vectors
calculate_pagerank_if_needed → _calculate_pagerank_if_needed

# World
build_context → _build_context
```

### Phase 2: Simplify Complex Parameters (30 min)
```python
# Teambook.read - Combine 9 boolean flags
def read(note_id, include=None, **kwargs):
    # include=['summary', 'edges', 'pagerank', 'ownership', ...]

# Notebook.recall - Combine 8 params
def recall(query, limit=10, options=None, **kwargs):
    # options={'pinned': True, 'summary': True, 'fuzzy': False, ...}
```

### Phase 3: Description Cleanup (10 min)
- Shorten descriptions >30 tokens
- Remove filler words

---

## Conclusion

**Current state**: 7,282 tokens per request

**After optimizations**: ~6,500 tokens per request (10.8% reduction)

**Annual savings**: ~28.7M tokens = ~$86/year

**Additional benefits**:
- Cleaner API surface (fewer exposed internals)
- Easier for AIs to select the right tools
- Better parameter ergonomics (arrays/dicts vs many booleans)
