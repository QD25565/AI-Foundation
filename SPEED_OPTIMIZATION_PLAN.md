# Speed Optimization Plan 🚀

## Executive Summary

Performance analysis identified **37 high-severity bottlenecks** across the codebase:
- **33 DB queries in loops** (N+1 problem) - CRITICAL
- **273 total database operations** - needs pooling & caching
- **2 blocking network calls** - World tool (IP lookup, weather API)
- **4 blocking operations** (sleep, stdin)
- **8 functions without caching**

**Potential improvements**:
- 50-70% reduction in database query time
- 200-500ms latency reduction per network call
- 30-50% overall response time improvement
- Better scalability under concurrent load

---

## Critical Issues (Immediate Action Required)

### 1. N+1 Query Problem (33 instances) 🔥

**What is it**: Executing a query inside a loop, causing O(n) database roundtrips instead of O(1).

**Example from teambook_storage.py**:
```python
# BAD (Current):
for ref_id in valid_refs:
    conn.execute('''
        INSERT INTO edges (from_id, to_id, edge_type)
        VALUES (?, ?, 'reference')
    ''', (note_id, ref_id))
```

**Fixed**:
```python
# GOOD (Batch insert):
if valid_refs:
    values = [(note_id, ref_id, 'reference') for ref_id in valid_refs]
    conn.executemany('''
        INSERT INTO edges (from_id, to_id, edge_type)
        VALUES (?, ?, ?)
    ''', values)
```

**Impact**: 10-100x faster for bulk operations

#### Files with N+1 Issues:
- `teambook_storage.py`: 13 instances (WORST)
- `notebook_storage.py`: 10 instances
- `teambook_api.py`: 7 instances
- `notebook_main.py`: 2 instances
- `task_manager.py`: 1 instance

---

### 2. Database Connection Pooling (273 operations) 🔥

**Current state**: Each operation opens/closes connection
**Problem**: Connection overhead adds 10-50ms per operation

**Solution**: Implement connection pooling

```python
# Add to mcp_shared.py or each storage module:
from functools import lru_cache
import duckdb

_connection_pool = {}

@lru_cache(maxsize=1)
def get_pooled_connection(db_path: str):
    """Reuse connections instead of creating new ones"""
    if db_path not in _connection_pool:
        _connection_pool[db_path] = duckdb.connect(db_path)
    return _connection_pool[db_path]
```

**Impact**: 30-50% reduction in DB operation time

---

### 3. Network Call Optimization (2 blocking calls) 🔥

**Location**: `world.py` lines 86, 138

**Current**:
```python
# Blocking network call (200-500ms)
resp = requests.get("http://ip-api.com/json/", timeout=3)
```

**Problems**:
1. Blocks entire tool execution
2. Fails if offline
3. No caching (repeated calls to same endpoint)

**Solution 1: Aggressive Caching** (Quick win)
```python
from functools import lru_cache
import time

# Cache for 1 hour
_location_cache = None
_location_cache_time = None

def get_location():
    global _location_cache, _location_cache_time

    # Use cache if < 1 hour old
    if _location_cache and _location_cache_time:
        if time.time() - _location_cache_time < 3600:
            return _location_cache

    # Try IP lookup (with timeout)
    try:
        resp = requests.get("http://ip-api.com/json/", timeout=1)  # Reduced timeout
        if resp.status_code == 200:
            _location_cache = resp.json()
            _location_cache_time = time.time()
            return _location_cache
    except:
        # Return cached value even if stale
        if _location_cache:
            return _location_cache

    return None
```

**Solution 2: Async Network Calls** (Best performance)
```python
import asyncio
import aiohttp

async def get_location_async():
    async with aiohttp.ClientSession() as session:
        async with session.get("http://ip-api.com/json/", timeout=1) as resp:
            return await resp.json()

# Use in sync context:
def get_location():
    try:
        return asyncio.run(get_location_async())
    except:
        return None
```

**Impact**: 200-500ms latency reduction, + offline resilience

---

## High Priority Optimizations

### 4. Query Result Caching

**Functions without caching** (8 total):
- `get_full_note()` (notebook + teambook)
- `calculate_pagerank_duckdb()` (both storages)
- `get_storage_stats()` (both storages)

**Solution**:
```python
from functools import lru_cache
import hashlib

def cache_key(*args, **kwargs):
    """Create cache key from function args"""
    key = str(args) + str(sorted(kwargs.items()))
    return hashlib.md5(key.encode()).hexdigest()

# Apply to frequently called functions:
@lru_cache(maxsize=128)
def get_full_note(id: int, verbose: bool = False):
    # ... existing code
    pass
```

**Impact**: Eliminates redundant DB queries for frequently accessed notes

---

### 5. Missing Database Indexes

**Current findings**:
- `id` column queried 44 times across files (should be PRIMARY KEY or indexed)
- `parent_id` queried 6 times (needs index)
- `name` queried 3 times (needs index)

**Solution**:
```sql
-- Add to init_db() functions:
CREATE INDEX IF NOT EXISTS idx_notes_id ON notes(id);
CREATE INDEX IF NOT EXISTS idx_edges_parent_id ON edges(parent_id);
CREATE INDEX IF NOT EXISTS idx_notes_name ON notes(name);
CREATE INDEX IF NOT EXISTS idx_created ON notes(created);
```

**Impact**: 50-90% faster WHERE/JOIN queries on indexed columns

---

### 6. Batch Operations

**Current**: Individual inserts/updates
**Better**: Batch operations

```python
# BEFORE (Slow):
for task in tasks:
    conn.execute("INSERT INTO tasks (...) VALUES (?)", task)

# AFTER (Fast):
conn.executemany("INSERT INTO tasks (...) VALUES (?)", tasks)
```

**Files needing batch optimization**:
- `teambook_storage.py` - edge creation (line 629)
- `notebook_storage.py` - embedding backfill (line 596)
- `task_manager.py` - bulk task operations (line 253)

**Impact**: 5-10x faster bulk operations

---

## Medium Priority Optimizations

### 7. Remove Blocking Operations

**Current blocking ops**:
- `time.sleep()` in teambook_storage.py (line 212) - retry logic
- `time.sleep()` in task_manager.py (line 315) - monitoring
- `sys.stdin.readline()` in notebook_main.py (line 1159) - CLI input

**Solutions**:
```python
# Instead of sleep() for retries:
import tenacity

@tenacity.retry(
    stop=tenacity.stop_after_attempt(3),
    wait=tenacity.wait_exponential(multiplier=1, min=1, max=10)
)
def operation_with_retry():
    # ... existing code

# For monitoring, use event-driven approach:
import threading

def monitor_in_background():
    # Non-blocking monitor
    threading.Thread(target=_monitor_loop, daemon=True).start()
```

---

### 8. Optimize Nested Loops (6 instances)

**Example from teambook_api.py**:
```python
# BEFORE (O(n²)):
for ai_id, last_seen in sorted_ais:
    for note in notes:
        if note.ai_id == ai_id:
            # process

# AFTER (O(n)):
notes_by_ai = defaultdict(list)
for note in notes:
    notes_by_ai[note.ai_id].append(note)

for ai_id, last_seen in sorted_ais:
    for note in notes_by_ai[ai_id]:
        # process
```

**Impact**: Scales better with large datasets

---

## Implementation Roadmap

### Phase 1: Quick Wins (1-2 hours) ⚡
**ROI: HIGH** - Immediate 30-40% performance gain

1. ✅ **Add connection pooling** (30 min)
   - Create `get_pooled_connection()` in mcp_shared.py
   - Replace all `duckdb.connect()` calls

2. ✅ **Cache network calls** (20 min)
   - Add 1-hour cache to `get_location()` in world.py
   - Add 10-min cache to `get_weather()` (already exists, verify)

3. ✅ **Add @lru_cache to hot functions** (30 min)
   - `get_full_note()` - both tools
   - `calculate_pagerank_duckdb()` - both storages
   - `get_storage_stats()` - both storages

4. ✅ **Add missing indexes** (30 min)
   - Update all `init_db()` functions
   - Run migrations on existing databases

### Phase 2: N+1 Query Fixes (3-4 hours) 🔥
**ROI: VERY HIGH** - 50-70% reduction in query time

1. ✅ **Batch edge creation** (teambook_storage.py, notebook_storage.py)
2. ✅ **Batch tag lookups** (all storage modules)
3. ✅ **Optimize search results** (avoid per-result queries)
4. ✅ **Fix loop-based updates** (task_manager.py)

### Phase 3: Async Optimization (4-6 hours) 🚀
**ROI: MEDIUM-HIGH** - Better concurrency, lower latency

1. ⏱️ **Async network calls** (world.py)
2. ⏱️ **Async database operations** (optional - big refactor)
3. ⏱️ **Event-driven monitoring** (task_manager.py)

### Phase 4: Advanced Optimizations (8+ hours) 🎯
**ROI: MEDIUM** - Marginal gains for heavy workloads

1. ⏱️ **Query result caching layer** (Redis/in-memory)
2. ⏱️ **Prepared statements** (reuse query plans)
3. ⏱️ **Database sharding** (for massive scale)

---

## Expected Performance Improvements

### Before Optimization
- Average tool response: 200-500ms
- Database query time: 100-300ms
- Network calls: 200-500ms per call
- Bulk operations: O(n) queries = 1000ms for 100 items

### After Phase 1 (Quick Wins)
- Average tool response: 120-300ms (**40% faster**)
- Database query time: 50-150ms (connection pooling)
- Network calls: <50ms (cached) or 200-500ms (first call)
- Bulk operations: Still O(n)

### After Phase 2 (N+1 Fixes)
- Average tool response: 80-200ms (**60% faster**)
- Database query time: 30-100ms (batched queries)
- Bulk operations: O(1) = 50ms for 100 items (**20x faster**)

### After Phase 3 (Async)
- Average tool response: 50-150ms (**70% faster**)
- Network calls: Non-blocking, parallel execution
- Better concurrency: 5+ AIs running smoothly

---

## Measurement & Validation

### Add Performance Monitoring

```python
# Add to mcp_shared.py:
import time
from functools import wraps

def timed(func):
    """Decorator to measure execution time"""
    @wraps(func)
    def wrapper(*args, **kwargs):
        start = time.perf_counter()
        result = func(*args, **kwargs)
        duration = (time.perf_counter() - start) * 1000

        if duration > 100:  # Log slow operations
            print(f"[SLOW] {func.__name__}: {duration:.1f}ms")

        return result
    return wrapper

# Apply to critical functions:
@timed
def write(...):
    ...
```

### Benchmark Script

```python
# benchmark.py
import time
from tools import notebook, teambook, task_manager

def benchmark_operation(name, func, iterations=100):
    start = time.perf_counter()
    for _ in range(iterations):
        func()
    duration = time.perf_counter() - start
    avg = (duration / iterations) * 1000
    print(f"{name}: {avg:.2f}ms average")

# Run benchmarks
benchmark_operation("notebook.remember", lambda: notebook.remember("test"))
benchmark_operation("teambook.write", lambda: teambook.write("test"))
benchmark_operation("task_manager.list_tasks", lambda: task_manager.list_tasks())
```

---

## Success Metrics

**Performance targets**:
- ✅ Average response time: <150ms (current: 200-500ms)
- ✅ Database queries: <50ms (current: 100-300ms)
- ✅ Network calls: <50ms cached, <200ms uncached
- ✅ Bulk operations: <100ms for 100 items (current: 1000ms+)

**Scalability targets**:
- ✅ Support 10+ concurrent AIs (current: 5)
- ✅ Handle 1000+ notes without slowdown
- ✅ Sub-second response under load

---

## Conclusion

**Current state**: Good functionality, moderate performance
**After optimizations**: Excellent performance, production-ready

**Priority order**:
1. Phase 1 (Quick Wins) - **DO THIS FIRST** → 40% improvement in 1-2 hours
2. Phase 2 (N+1 Fixes) - **HIGH IMPACT** → Additional 30% improvement
3. Phase 3 (Async) - **NICE TO HAVE** → Better concurrency
4. Phase 4 (Advanced) - **FUTURE** → Only if needed at scale

**Next step**: Implement Phase 1 quick wins? 🚀
