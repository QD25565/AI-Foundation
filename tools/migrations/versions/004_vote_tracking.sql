-- Migration 004: Vote Tracking System
-- Description: Track active votes in teambook for awareness injection
-- Author: LYRA-601
-- Date: 2025-11-05
-- Status: Phase 3 - Vote Tracking

-- ============================================================================
-- VOTES TABLE
-- ============================================================================
-- Stores active votes with topic, options, and status

CREATE TABLE IF NOT EXISTS votes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    topic TEXT NOT NULL,                    -- Vote topic/question
    options TEXT NOT NULL,                  -- JSON array of options: ["Option A", "Option B"]
    status TEXT NOT NULL DEFAULT 'open',   -- 'open' or 'closed'
    created_by TEXT NOT NULL,              -- AI who created vote
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    closed_at TIMESTAMP,                   -- When vote was closed
    total_voters INTEGER NOT NULL DEFAULT 4,  -- Expected number of voters
    UNIQUE(topic, created_at)              -- Prevent duplicate votes
);

CREATE INDEX IF NOT EXISTS idx_votes_status
    ON votes(status);

CREATE INDEX IF NOT EXISTS idx_votes_created
    ON votes(created_at DESC);


-- ============================================================================
-- VOTE_RESPONSES TABLE
-- ============================================================================
-- Stores individual AI votes on topics

CREATE TABLE IF NOT EXISTS vote_responses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vote_id INTEGER NOT NULL,              -- FK to votes.id
    voter_ai TEXT NOT NULL,                -- AI who cast vote
    choice TEXT NOT NULL,                  -- Their selected option
    voted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (vote_id) REFERENCES votes(id) ON DELETE CASCADE,
    UNIQUE(vote_id, voter_ai)              -- One vote per AI per topic
);

CREATE INDEX IF NOT EXISTS idx_vote_responses_vote
    ON vote_responses(vote_id);

CREATE INDEX IF NOT EXISTS idx_vote_responses_voter
    ON vote_responses(voter_ai);


-- ============================================================================
-- VOTE_STATS VIEW
-- ============================================================================
-- Quick stats on active votes

CREATE VIEW IF NOT EXISTS vote_stats AS
SELECT
    v.id,
    v.topic,
    v.status,
    v.created_by,
    v.created_at,
    v.total_voters,
    COUNT(vr.id) as votes_cast,
    (v.total_voters - COUNT(vr.id)) as votes_pending,
    ROUND(CAST(COUNT(vr.id) AS FLOAT) / v.total_voters * 100, 1) as completion_pct
FROM votes v
LEFT JOIN vote_responses vr ON v.id = vr.vote_id
GROUP BY v.id;


-- ============================================================================
-- VOTE_DETAILS VIEW
-- ============================================================================
-- Detailed vote breakdown by option

CREATE VIEW IF NOT EXISTS vote_details AS
SELECT
    v.id as vote_id,
    v.topic,
    v.status,
    vr.choice,
    COUNT(vr.id) as vote_count,
    GROUP_CONCAT(vr.voter_ai, ', ') as voters
FROM votes v
LEFT JOIN vote_responses vr ON v.id = vr.vote_id
GROUP BY v.id, vr.choice;
