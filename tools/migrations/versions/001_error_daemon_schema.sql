-- Migration: 001_error_daemon_schema
-- Description: Create Error Daemon tables for world-class error tracking
-- Author: Resonance-403
-- Date: 2025-11-04

-- UP
-- ============================================================================
-- Error Daemon Schema - Sentry/DataDog Style Error Tracking
-- ============================================================================

-- Main error issues table (deduplicated errors)
CREATE TABLE IF NOT EXISTS error_issues (
    id SERIAL PRIMARY KEY,
    fingerprint VARCHAR(16) UNIQUE NOT NULL,  -- SHA-256 hash (first 16 chars)
    error_type VARCHAR(255) NOT NULL,         -- ValueError, KeyError, etc.
    error_message TEXT NOT NULL,              -- Normalized message
    stack_signature VARCHAR(500) NOT NULL,    -- Top 5 stack frames
    full_stack_trace TEXT,                    -- Complete stack trace (first occurrence)

    -- Status tracking
    status VARCHAR(20) DEFAULT 'active',      -- active, resolved, ignored, regressed
    severity VARCHAR(20) DEFAULT 'error',     -- critical, error, warning

    -- Occurrence tracking
    first_seen TIMESTAMPTZ NOT NULL,
    last_seen TIMESTAMPTZ NOT NULL,
    occurrence_count INTEGER DEFAULT 1,

    -- AI tracking
    first_seen_by VARCHAR(100),               -- AI ID that first encountered this
    affected_ais TEXT[],                      -- Array of AI IDs that hit this error

    -- Function tracking (via Code-Graph integration)
    affected_functions TEXT[],                -- Qualified function names
    affected_files TEXT[],                    -- File paths

    -- Metadata
    tags TEXT[],                              -- User-defined tags
    notes TEXT,                               -- Human-added notes
    resolved_at TIMESTAMPTZ,
    resolved_by VARCHAR(100),
    resolution_notes TEXT,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for fast lookups
CREATE INDEX IF NOT EXISTS idx_error_issues_fingerprint ON error_issues(fingerprint);
CREATE INDEX IF NOT EXISTS idx_error_issues_status ON error_issues(status);
CREATE INDEX IF NOT EXISTS idx_error_issues_last_seen ON error_issues(last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_error_issues_severity ON error_issues(severity);
CREATE INDEX IF NOT EXISTS idx_error_issues_error_type ON error_issues(error_type);
CREATE INDEX IF NOT EXISTS idx_error_issues_affected_files ON error_issues USING GIN(affected_files);

COMMENT ON TABLE error_issues IS 'Deduplicated error tracking (one row per unique error)';
COMMENT ON COLUMN error_issues.fingerprint IS 'SHA-256 hash for deduplication (first 16 hex chars)';
COMMENT ON COLUMN error_issues.occurrence_count IS 'Total number of times this error occurred';

-- Individual error occurrences (for trend analysis)
CREATE TABLE IF NOT EXISTS error_occurrences (
    id SERIAL PRIMARY KEY,
    issue_id INTEGER REFERENCES error_issues(id) ON DELETE CASCADE,

    -- Occurrence details
    timestamp TIMESTAMPTZ NOT NULL,
    ai_id VARCHAR(100) NOT NULL,

    -- Tracing context (OpenTelemetry integration)
    trace_id VARCHAR(64),                     -- OpenTelemetry trace ID
    span_id VARCHAR(32),                      -- OpenTelemetry span ID

    -- Full context
    full_stack_trace TEXT,                    -- Complete stack for this occurrence
    error_context JSONB,                      -- Additional context (locals, args, etc.)

    -- Environment
    working_directory TEXT,
    file_path TEXT,
    function_name VARCHAR(255),
    line_number INTEGER,

    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for occurrence queries
CREATE INDEX IF NOT EXISTS idx_error_occurrences_issue_id ON error_occurrences(issue_id);
CREATE INDEX IF NOT EXISTS idx_error_occurrences_timestamp ON error_occurrences(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_error_occurrences_ai_id ON error_occurrences(ai_id);
CREATE INDEX IF NOT EXISTS idx_error_occurrences_trace_id ON error_occurrences(trace_id);

COMMENT ON TABLE error_occurrences IS 'Individual error occurrences (one row per error event)';
COMMENT ON COLUMN error_occurrences.trace_id IS 'OpenTelemetry trace ID for distributed debugging';

-- Aggregated trends (hourly/daily rollups for charting)
CREATE TABLE IF NOT EXISTS error_trends (
    id SERIAL PRIMARY KEY,
    fingerprint VARCHAR(16) NOT NULL,         -- References error_issues.fingerprint
    time_bucket TIMESTAMPTZ NOT NULL,         -- Hour or day boundary
    occurrence_count INTEGER NOT NULL,        -- Count in this time bucket
    unique_ais INTEGER NOT NULL,              -- Number of unique AIs affected
    severity_distribution JSONB,              -- {"critical": 2, "error": 5, "warning": 1}

    created_at TIMESTAMPTZ DEFAULT NOW(),

    -- Unique constraint: one row per fingerprint per time bucket
    UNIQUE(fingerprint, time_bucket)
);

-- Indexes for trend queries
CREATE INDEX IF NOT EXISTS idx_error_trends_fingerprint ON error_trends(fingerprint);
CREATE INDEX IF NOT EXISTS idx_error_trends_time_bucket ON error_trends(time_bucket DESC);
CREATE INDEX IF NOT EXISTS idx_error_trends_fingerprint_time ON error_trends(fingerprint, time_bucket);

COMMENT ON TABLE error_trends IS 'Aggregated error trends for charting (hourly/daily rollups)';
COMMENT ON COLUMN error_trends.time_bucket IS 'Truncated to hour or day for aggregation';

-- Error context (rich context for specific errors)
CREATE TABLE IF NOT EXISTS error_context (
    id SERIAL PRIMARY KEY,
    occurrence_id INTEGER REFERENCES error_occurrences(id) ON DELETE CASCADE,

    -- Context types
    context_type VARCHAR(50) NOT NULL,        -- locals, args, env, config, etc.
    context_data JSONB NOT NULL,              -- Actual context data

    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for context lookups
CREATE INDEX IF NOT EXISTS idx_error_context_occurrence_id ON error_context(occurrence_id);
CREATE INDEX IF NOT EXISTS idx_error_context_type ON error_context(context_type);

COMMENT ON TABLE error_context IS 'Rich context for error debugging (locals, args, environment)';

-- DOWN
-- ============================================================================
-- Rollback: Drop all Error Daemon tables
-- ============================================================================

DROP TABLE IF EXISTS error_context CASCADE;
DROP TABLE IF EXISTS error_trends CASCADE;
DROP TABLE IF EXISTS error_occurrences CASCADE;
DROP TABLE IF EXISTS error_issues CASCADE;
