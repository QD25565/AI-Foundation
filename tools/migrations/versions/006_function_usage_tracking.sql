-- Migration 006: Function Usage Tracking
-- ========================================
-- Tracks function calls, performance, and usage patterns per AI
--
-- Purpose:
-- - Understand which functions are used (discoverability)
-- - Identify unused functions (deprecation candidates)
-- - Measure performance (optimization targets)
-- - Per-AI usage patterns (personalization opportunities)
--
-- Usage: psql $POSTGRES_URL -f 006_function_usage_tracking.sql

-- ================================
-- Table: function_usage
-- ================================
-- Aggregated metrics per (function, module, AI) combination

CREATE TABLE IF NOT EXISTS function_usage (
    id SERIAL PRIMARY KEY,

    -- Identification
    function_name TEXT NOT NULL,     -- e.g., "broadcast", "remember", "create_project"
    module_name TEXT NOT NULL,       -- e.g., "teambook", "notebook", "project_service"
    ai_id TEXT NOT NULL,             -- e.g., "sage-386", "cascade-623-731"

    -- Aggregated Call Metrics
    call_count BIGINT DEFAULT 0,     -- Total number of calls
    success_count BIGINT DEFAULT 0,  -- Successful calls (no exceptions)
    error_count BIGINT DEFAULT 0,    -- Failed calls (exceptions raised)

    -- Performance Metrics (milliseconds)
    total_duration_ms DOUBLE PRECISION DEFAULT 0,  -- Sum of all durations
    avg_duration_ms DOUBLE PRECISION DEFAULT 0,    -- Average duration
    min_duration_ms DOUBLE PRECISION,              -- Fastest call
    max_duration_ms DOUBLE PRECISION,              -- Slowest call

    -- Temporal Tracking
    first_used TIMESTAMPTZ DEFAULT NOW(),   -- First call timestamp
    last_used TIMESTAMPTZ DEFAULT NOW(),    -- Most recent call

    -- Metadata
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    -- Constraints: One row per (function, module, ai) combination
    UNIQUE(function_name, module_name, ai_id)
);

-- Performance Indexes
CREATE INDEX IF NOT EXISTS idx_usage_function ON function_usage(function_name);
CREATE INDEX IF NOT EXISTS idx_usage_ai ON function_usage(ai_id);
CREATE INDEX IF NOT EXISTS idx_usage_module ON function_usage(module_name);
CREATE INDEX IF NOT EXISTS idx_usage_last_used ON function_usage(last_used DESC);
CREATE INDEX IF NOT EXISTS idx_usage_call_count ON function_usage(call_count DESC);
CREATE INDEX IF NOT EXISTS idx_usage_avg_duration ON function_usage(avg_duration_ms DESC);

-- Comments for Documentation
COMMENT ON TABLE function_usage IS 'Aggregated function usage metrics per (function, module, AI)';
COMMENT ON COLUMN function_usage.function_name IS 'Function name (e.g., broadcast, remember)';
COMMENT ON COLUMN function_usage.module_name IS 'Module name (e.g., teambook, notebook)';
COMMENT ON COLUMN function_usage.ai_id IS 'AI crypto ID (e.g., sage-386)';
COMMENT ON COLUMN function_usage.call_count IS 'Total calls (success + error)';
COMMENT ON COLUMN function_usage.avg_duration_ms IS 'Average call duration in milliseconds';
COMMENT ON COLUMN function_usage.last_used IS 'Most recent call timestamp (for staleness detection)';

-- ================================
-- Analytics Views
-- ================================

-- View: Top Functions (Most Used)
CREATE OR REPLACE VIEW top_functions AS
SELECT
    function_name,
    module_name,
    SUM(call_count) as total_calls,
    SUM(success_count) as total_success,
    SUM(error_count) as total_errors,
    AVG(avg_duration_ms) as avg_duration_ms,
    MAX(last_used) as last_used
FROM function_usage
GROUP BY function_name, module_name
ORDER BY total_calls DESC
LIMIT 20;

COMMENT ON VIEW top_functions IS 'Top 20 most-used functions across all AIs';

-- View: Unused Functions (Last 7 Days)
CREATE OR REPLACE VIEW unused_functions AS
SELECT
    function_name,
    module_name,
    MAX(last_used) as last_used,
    EXTRACT(EPOCH FROM (NOW() - MAX(last_used))) / 86400 as days_since_use
FROM function_usage
GROUP BY function_name, module_name
HAVING MAX(last_used) < NOW() - INTERVAL '7 days'
ORDER BY last_used ASC;

COMMENT ON VIEW unused_functions IS 'Functions not used in last 7 days (deprecation candidates)';

-- View: Slow Functions (Avg > 1000ms)
CREATE OR REPLACE VIEW slow_functions AS
SELECT
    function_name,
    module_name,
    SUM(call_count) as total_calls,
    AVG(avg_duration_ms) as avg_duration_ms,
    MAX(max_duration_ms) as max_duration_ms
FROM function_usage
GROUP BY function_name, module_name
HAVING AVG(avg_duration_ms) > 1000
ORDER BY avg_duration_ms DESC;

COMMENT ON VIEW slow_functions IS 'Functions with average duration > 1000ms (optimization targets)';

-- View: Per-AI Usage Summary
CREATE OR REPLACE VIEW ai_usage_summary AS
SELECT
    ai_id,
    COUNT(DISTINCT function_name) as unique_functions_used,
    SUM(call_count) as total_calls,
    SUM(error_count) as total_errors,
    AVG(avg_duration_ms) as overall_avg_duration_ms,
    MAX(last_used) as most_recent_activity
FROM function_usage
GROUP BY ai_id
ORDER BY total_calls DESC;

COMMENT ON VIEW ai_usage_summary IS 'Per-AI usage statistics (function coverage, call volume)';

-- ================================
-- Analytics Functions
-- ================================

-- Function: Get function usage trends (last N days)
CREATE OR REPLACE FUNCTION get_function_trend(
    func_name TEXT,
    mod_name TEXT,
    days_back INTEGER DEFAULT 30
)
RETURNS TABLE(
    ai_id TEXT,
    call_count BIGINT,
    avg_duration_ms DOUBLE PRECISION,
    last_used TIMESTAMPTZ
) AS $$
BEGIN
    RETURN QUERY
    SELECT
        fu.ai_id,
        fu.call_count,
        fu.avg_duration_ms,
        fu.last_used
    FROM function_usage fu
    WHERE fu.function_name = func_name
      AND fu.module_name = mod_name
      AND fu.last_used > NOW() - make_interval(days => days_back)
    ORDER BY fu.call_count DESC;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION get_function_trend IS 'Get per-AI usage trend for specific function';

-- ================================
-- Verification Queries
-- ================================

-- Verify migration applied
DO $$
BEGIN
    RAISE NOTICE 'Migration 006: Function Usage Tracking - COMPLETE';
    RAISE NOTICE 'Tables created: function_usage';
    RAISE NOTICE 'Views created: top_functions, unused_functions, slow_functions, ai_usage_summary';
    RAISE NOTICE 'Functions created: get_function_trend';
    RAISE NOTICE 'Ready for @track_usage decorator integration';
END $$;
