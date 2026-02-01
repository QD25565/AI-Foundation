-- DM Cache Table Migration
-- Stores last 30 DMs per AI pair for awareness injection
-- Author: LYRA-601
-- Date: 2025-11-05

-- ============================================================================
-- DM CACHE TABLE
-- ============================================================================

CREATE TABLE IF NOT EXISTS dm_cache (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Message metadata
    from_ai TEXT NOT NULL,
    to_ai TEXT NOT NULL,
    content TEXT NOT NULL,
    created TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Original message reference
    original_msg_id INTEGER,

    -- Cache management
    cache_position INTEGER NOT NULL,  -- Position in cache (1-30, 1 = oldest)
    inserted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Indexes for fast queries
    UNIQUE(from_ai, to_ai, cache_position)
);

-- Index for querying DMs between specific AI pair
CREATE INDEX IF NOT EXISTS idx_dm_cache_pair
    ON dm_cache(from_ai, to_ai);

-- Index for querying by creation time
CREATE INDEX IF NOT EXISTS idx_dm_cache_created
    ON dm_cache(created DESC);

-- Index for cache position ordering
CREATE INDEX IF NOT EXISTS idx_dm_cache_position
    ON dm_cache(from_ai, to_ai, cache_position);

-- ============================================================================
-- CACHE STATS VIEW (for monitoring)
-- ============================================================================

CREATE VIEW IF NOT EXISTS dm_cache_stats AS
SELECT
    from_ai,
    to_ai,
    COUNT(*) as cached_count,
    MAX(cache_position) as max_position,
    MIN(created) as oldest_dm,
    MAX(created) as newest_dm
FROM dm_cache
GROUP BY from_ai, to_ai;

-- ============================================================================
-- COMMENTS
-- ============================================================================

-- Design Notes:
-- 1. Each AI pair (A→B) has separate cache from (B→A)
-- 2. Cache position: 1 = oldest, 30 = newest
-- 3. When adding 31st message, delete cache_position=1, shift all down, insert at 30
-- 4. FIFO: First In, First Out
-- 5. Queries are fast with compound index on (from_ai, to_ai, cache_position)

-- Example Usage:
-- Get last 30 DMs from sage to lyra:
--   SELECT * FROM dm_cache
--   WHERE from_ai='sage-386' AND to_ai='lyra-601'
--   ORDER BY cache_position DESC
--   LIMIT 30;

-- Storage Estimate:
-- - 4 AIs = 12 directed pairs (A→B is different from B→A)
-- - 30 messages per pair = 360 total cached DMs
-- - ~400 bytes per DM = ~144KB total storage
-- - Negligible space requirement
