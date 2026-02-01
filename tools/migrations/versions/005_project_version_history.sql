-- Migration: Project Version History
-- Date: 2025-11-06
-- Purpose: Track historical changes to projects and features for evolution tracking
-- Author: RESONANCE-403

-- ============================================
-- PROJECT VERSIONS TABLE
-- ============================================

CREATE TABLE IF NOT EXISTS project_versions (
    id SERIAL PRIMARY KEY,
    project_id INTEGER NOT NULL,
    version_number INTEGER NOT NULL,
    name TEXT,
    overview TEXT,
    details TEXT,
    root_directory TEXT,
    status TEXT,
    config JSONB,
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    change_summary TEXT,  -- Optional: What changed in this version
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_project_versions_project_id ON project_versions(project_id);
CREATE INDEX IF NOT EXISTS idx_project_versions_updated_at ON project_versions(updated_at DESC);

COMMENT ON TABLE project_versions IS 'Historical versions of project configurations';
COMMENT ON COLUMN project_versions.version_number IS 'Sequential version number (1, 2, 3...)';
COMMENT ON COLUMN project_versions.change_summary IS 'Human-readable summary of what changed';

-- ============================================
-- PROJECT FEATURE VERSIONS TABLE
-- ============================================

CREATE TABLE IF NOT EXISTS project_feature_versions (
    id SERIAL PRIMARY KEY,
    feature_id INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    version_number INTEGER NOT NULL,
    name TEXT,
    overview TEXT,
    details TEXT,
    directory TEXT,
    status TEXT,
    config JSONB,
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    change_summary TEXT,
    FOREIGN KEY (feature_id) REFERENCES project_features(id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_project_feature_versions_feature_id ON project_feature_versions(feature_id);
CREATE INDEX IF NOT EXISTS idx_project_feature_versions_project_id ON project_feature_versions(project_id);
CREATE INDEX IF NOT EXISTS idx_project_feature_versions_updated_at ON project_feature_versions(updated_at DESC);

COMMENT ON TABLE project_feature_versions IS 'Historical versions of project feature configurations';
COMMENT ON COLUMN project_feature_versions.version_number IS 'Sequential version number (1, 2, 3...)';

-- ============================================
-- HELPER FUNCTION: Get Latest Version Number
-- ============================================

CREATE OR REPLACE FUNCTION get_next_project_version(p_project_id INTEGER)
RETURNS INTEGER AS $$
DECLARE
    next_version INTEGER;
BEGIN
    SELECT COALESCE(MAX(version_number), 0) + 1
    INTO next_version
    FROM project_versions
    WHERE project_id = p_project_id;

    RETURN next_version;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION get_next_feature_version(p_feature_id INTEGER)
RETURNS INTEGER AS $$
DECLARE
    next_version INTEGER;
BEGIN
    SELECT COALESCE(MAX(version_number), 0) + 1
    INTO next_version
    FROM project_feature_versions
    WHERE feature_id = p_feature_id;

    RETURN next_version;
END;
$$ LANGUAGE plpgsql;

-- ============================================
-- HELPER VIEW: Latest Versions Summary
-- ============================================

CREATE OR REPLACE VIEW project_version_summary AS
SELECT
    p.id as project_id,
    p.name,
    COUNT(pv.id) as total_versions,
    MAX(pv.version_number) as latest_version,
    MAX(pv.updated_at) as last_modified,
    MAX(pv.updated_by) as last_modified_by
FROM projects p
LEFT JOIN project_versions pv ON p.id = pv.project_id
GROUP BY p.id, p.name;

COMMENT ON VIEW project_version_summary IS 'Summary of project version counts and metadata';

-- ============================================
-- SUCCESS MESSAGE
-- ============================================

DO $$
BEGIN
    RAISE NOTICE 'Migration 005: Project version history tables created successfully';
END $$;
