-- Migration 006: Enhanced Metrics System
-- Date: 2025-11-06
-- Purpose: Add per-AI tracking, tool coverage, and performance baselines
-- Author: RESONANCE-403

-- ============================================================================
-- PART 1: Enhance existing metrics_function_calls table
-- ============================================================================

-- Add new columns for better tracking
ALTER TABLE metrics_function_calls
ADD COLUMN IF NOT EXISTS ai_id TEXT,
ADD COLUMN IF NOT EXISTS session_id TEXT,
ADD COLUMN IF NOT EXISTS tool_name TEXT,
ADD COLUMN IF NOT EXISTS baseline_duration_ms REAL,
ADD COLUMN IF NOT EXISTS performance_delta REAL;

-- Add indexes for common queries
CREATE INDEX IF NOT EXISTS idx_metrics_function_calls_ai_id
    ON metrics_function_calls(ai_id);

CREATE INDEX IF NOT EXISTS idx_metrics_function_calls_tool_name
    ON metrics_function_calls(tool_name);

CREATE INDEX IF NOT EXISTS idx_metrics_function_calls_timestamp
    ON metrics_function_calls(timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_metrics_function_calls_ai_tool
    ON metrics_function_calls(ai_id, tool_name);

-- ============================================================================
-- PART 2: Tool Coverage Table
-- ============================================================================

CREATE TABLE IF NOT EXISTS tool_coverage (
    id SERIAL PRIMARY KEY,
    tool_name TEXT NOT NULL,
    function_name TEXT NOT NULL,
    total_calls INTEGER DEFAULT 0,
    unique_ais INTEGER DEFAULT 0,
    last_called_at TIMESTAMP WITH TIME ZONE,
    average_duration_ms REAL,
    p95_duration_ms REAL,
    p99_duration_ms REAL,
    error_rate REAL DEFAULT 0.0,
    success_rate REAL DEFAULT 1.0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tool_name, function_name)
);

CREATE INDEX IF NOT EXISTS idx_tool_coverage_tool_name
    ON tool_coverage(tool_name);

CREATE INDEX IF NOT EXISTS idx_tool_coverage_total_calls
    ON tool_coverage(total_calls DESC);

CREATE INDEX IF NOT EXISTS idx_tool_coverage_last_called
    ON tool_coverage(last_called_at DESC NULLS LAST);

-- ============================================================================
-- PART 3: Per-AI Usage Table
-- ============================================================================

CREATE TABLE IF NOT EXISTS ai_tool_usage (
    id SERIAL PRIMARY KEY,
    ai_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    function_name TEXT NOT NULL,
    call_count INTEGER DEFAULT 0,
    total_duration_ms REAL DEFAULT 0,
    average_duration_ms REAL,
    last_used_at TIMESTAMP WITH TIME ZONE,
    first_used_at TIMESTAMP WITH TIME ZONE,
    error_count INTEGER DEFAULT 0,
    success_count INTEGER DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(ai_id, tool_name, function_name)
);

CREATE INDEX IF NOT EXISTS idx_ai_tool_usage_ai_id
    ON ai_tool_usage(ai_id);

CREATE INDEX IF NOT EXISTS idx_ai_tool_usage_tool_name
    ON ai_tool_usage(tool_name);

CREATE INDEX IF NOT EXISTS idx_ai_tool_usage_call_count
    ON ai_tool_usage(call_count DESC);

-- ============================================================================
-- PART 4: Performance Baselines Table
-- ============================================================================

CREATE TABLE IF NOT EXISTS performance_baselines (
    id SERIAL PRIMARY KEY,
    tool_name TEXT NOT NULL,
    function_name TEXT NOT NULL,
    operation_type TEXT,  -- e.g., 'read_10mb', 'search_1000_items', 'query_empty_db'
    baseline_p50_ms REAL NOT NULL,
    baseline_p95_ms REAL NOT NULL,
    baseline_p99_ms REAL NOT NULL,
    baseline_max_ms REAL,
    sample_size INTEGER NOT NULL,
    dataset_description TEXT,  -- What test data was used
    environment_notes TEXT,     -- CPU, RAM, PostgreSQL version, etc.
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(tool_name, function_name, operation_type)
);

CREATE INDEX IF NOT EXISTS idx_performance_baselines_tool_function
    ON performance_baselines(tool_name, function_name);

-- ============================================================================
-- PART 5: Materialized Views for Fast Queries
-- ============================================================================

-- View: Tool usage summary (refreshed hourly)
CREATE MATERIALIZED VIEW IF NOT EXISTS tool_usage_summary AS
SELECT
    tool_name,
    function_name,
    COUNT(*) as total_calls,
    COUNT(DISTINCT ai_id) as unique_ais,
    AVG(duration_ms) as avg_duration_ms,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) as p95_duration_ms,
    MAX(timestamp) as last_called_at,
    SUM(CASE WHEN success THEN 1 ELSE 0 END)::FLOAT / COUNT(*) as success_rate
FROM metrics_function_calls
WHERE timestamp > NOW() - INTERVAL '30 days'
  AND tool_name IS NOT NULL
GROUP BY tool_name, function_name;

CREATE INDEX IF NOT EXISTS idx_tool_usage_summary_tool
    ON tool_usage_summary(tool_name);

-- View: Per-AI usage summary
CREATE MATERIALIZED VIEW IF NOT EXISTS ai_usage_summary AS
SELECT
    ai_id,
    tool_name,
    COUNT(*) as total_calls,
    AVG(duration_ms) as avg_duration_ms,
    MAX(timestamp) as last_used_at,
    MIN(timestamp) as first_used_at
FROM metrics_function_calls
WHERE timestamp > NOW() - INTERVAL '30 days'
  AND ai_id IS NOT NULL
  AND tool_name IS NOT NULL
GROUP BY ai_id, tool_name;

CREATE INDEX IF NOT EXISTS idx_ai_usage_summary_ai
    ON ai_usage_summary(ai_id);

-- ============================================================================
-- PART 6: Helper Functions
-- ============================================================================

-- Function to refresh materialized views
CREATE OR REPLACE FUNCTION refresh_metrics_views()
RETURNS void AS $$
BEGIN
    REFRESH MATERIALIZED VIEW CONCURRENTLY tool_usage_summary;
    REFRESH MATERIALIZED VIEW CONCURRENTLY ai_usage_summary;
END;
$$ LANGUAGE plpgsql;

-- Function to update tool_coverage table (call after batch inserts)
CREATE OR REPLACE FUNCTION update_tool_coverage()
RETURNS void AS $$
BEGIN
    INSERT INTO tool_coverage (
        tool_name,
        function_name,
        total_calls,
        unique_ais,
        last_called_at,
        average_duration_ms,
        p95_duration_ms,
        p99_duration_ms,
        error_rate,
        success_rate,
        updated_at
    )
    SELECT
        tool_name,
        function_name,
        COUNT(*) as total_calls,
        COUNT(DISTINCT ai_id) as unique_ais,
        MAX(timestamp) as last_called_at,
        AVG(duration_ms) as average_duration_ms,
        PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) as p95_duration_ms,
        PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY duration_ms) as p99_duration_ms,
        1.0 - (SUM(CASE WHEN success THEN 1 ELSE 0 END)::FLOAT / COUNT(*)) as error_rate,
        SUM(CASE WHEN success THEN 1 ELSE 0 END)::FLOAT / COUNT(*) as success_rate,
        NOW() as updated_at
    FROM metrics_function_calls
    WHERE timestamp > NOW() - INTERVAL '30 days'
      AND tool_name IS NOT NULL
    GROUP BY tool_name, function_name
    ON CONFLICT (tool_name, function_name)
    DO UPDATE SET
        total_calls = EXCLUDED.total_calls,
        unique_ais = EXCLUDED.unique_ais,
        last_called_at = EXCLUDED.last_called_at,
        average_duration_ms = EXCLUDED.average_duration_ms,
        p95_duration_ms = EXCLUDED.p95_duration_ms,
        p99_duration_ms = EXCLUDED.p99_duration_ms,
        error_rate = EXCLUDED.error_rate,
        success_rate = EXCLUDED.success_rate,
        updated_at = NOW();
END;
$$ LANGUAGE plpgsql;

-- Function to update ai_tool_usage table
CREATE OR REPLACE FUNCTION update_ai_tool_usage()
RETURNS void AS $$
BEGIN
    INSERT INTO ai_tool_usage (
        ai_id,
        tool_name,
        function_name,
        call_count,
        total_duration_ms,
        average_duration_ms,
        last_used_at,
        first_used_at,
        error_count,
        success_count,
        updated_at
    )
    SELECT
        ai_id,
        tool_name,
        function_name,
        COUNT(*) as call_count,
        SUM(duration_ms) as total_duration_ms,
        AVG(duration_ms) as average_duration_ms,
        MAX(timestamp) as last_used_at,
        MIN(timestamp) as first_used_at,
        SUM(CASE WHEN NOT success THEN 1 ELSE 0 END) as error_count,
        SUM(CASE WHEN success THEN 1 ELSE 0 END) as success_count,
        NOW() as updated_at
    FROM metrics_function_calls
    WHERE timestamp > NOW() - INTERVAL '30 days'
      AND ai_id IS NOT NULL
      AND tool_name IS NOT NULL
    GROUP BY ai_id, tool_name, function_name
    ON CONFLICT (ai_id, tool_name, function_name)
    DO UPDATE SET
        call_count = EXCLUDED.call_count,
        total_duration_ms = EXCLUDED.total_duration_ms,
        average_duration_ms = EXCLUDED.average_duration_ms,
        last_used_at = EXCLUDED.last_used_at,
        error_count = EXCLUDED.error_count,
        success_count = EXCLUDED.success_count,
        updated_at = NOW();
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- PART 7: Verification Queries
-- ============================================================================

-- Check migration success
DO $$
DECLARE
    col_count INTEGER;
    table_count INTEGER;
BEGIN
    -- Check new columns added
    SELECT COUNT(*) INTO col_count
    FROM information_schema.columns
    WHERE table_name = 'metrics_function_calls'
      AND column_name IN ('ai_id', 'session_id', 'tool_name', 'baseline_duration_ms', 'performance_delta');

    IF col_count < 5 THEN
        RAISE EXCEPTION 'Migration failed: Not all columns added to metrics_function_calls';
    END IF;

    -- Check new tables created
    SELECT COUNT(*) INTO table_count
    FROM information_schema.tables
    WHERE table_name IN ('tool_coverage', 'ai_tool_usage', 'performance_baselines');

    IF table_count < 3 THEN
        RAISE EXCEPTION 'Migration failed: Not all tables created';
    END IF;

    RAISE NOTICE 'Migration 006: Enhanced Metrics - SUCCESS';
    RAISE NOTICE '  - Added 5 columns to metrics_function_calls';
    RAISE NOTICE '  - Created 3 new tables (tool_coverage, ai_tool_usage, performance_baselines)';
    RAISE NOTICE '  - Created 2 materialized views';
    RAISE NOTICE '  - Added 11 indexes';
    RAISE NOTICE '  - Created 3 helper functions';
END $$;

-- End of migration
