//! Notebook Compatibility Layer
//!
//! Wraps Engram to provide API compatibility with the old notebook_core interface.
//! Missing features return errors - they can be implemented as needed.

// Many types and methods here are WIP compatibility stubs - suppress dead_code for this module.
#![allow(dead_code)]

use anyhow::{anyhow, Result};
use engram::{Engram, Note as EngramNote};
use std::path::Path;

/// Note type compatible with old API
#[derive(Debug, Clone)]
pub struct Note {
    pub id: u64,
    pub content: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub timestamp: i64,
    pub priority: NotePriority,
    pub created: chrono::DateTime<chrono::Utc>,
}

impl Note {
    /// Create a new note (for insertion - id will be assigned by storage)
    pub fn new(content: String, tags: Vec<String>) -> Self {
        Self {
            id: 0,
            content,
            tags,
            pinned: false,
            timestamp: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            priority: NotePriority::Normal,
            created: chrono::Utc::now(),
        }
    }
}

impl From<EngramNote> for Note {
    fn from(n: EngramNote) -> Self {
        let created = chrono::DateTime::from_timestamp_nanos(n.timestamp);
        Self {
            id: n.id,
            content: n.content,
            tags: n.tags,
            pinned: n.pinned,
            timestamp: n.timestamp,
            priority: NotePriority::Normal,
            created,
        }
    }
}

/// Note priority (stub - not implemented in Engram)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePriority {
    Low,
    Normal,
    High,
    Critical,
}

/// Recall result
#[derive(Debug, Clone)]
pub struct RecallResult {
    pub note: Note,
    pub score: f32,
    pub final_score: f32,
    pub semantic_score: f32,
    pub keyword_score: f32,
    pub graph_score: f32,
}

/// Entity (stub)
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub properties: std::collections::HashMap<String, String>,
    pub aliases: Vec<String>,
    pub confidence: f32,
}

/// Strategy (stub)
#[derive(Debug, Clone)]
pub struct Strategy {
    pub id: String,
    pub title: String,
    pub context: String,
    pub approach: String,
    pub tags: Vec<String>,
    pub effectiveness: f32,
}

/// Insight (stub)
#[derive(Debug, Clone)]
pub struct Insight {
    pub id: String,
    pub discovery: String,
    pub tags: Vec<String>,
    pub confidence: f32,
}

/// Pattern (stub)
#[derive(Debug, Clone)]
pub struct Pattern {
    pub id: String,
    pub situation: String,
    pub pattern: String,
    pub tags: Vec<String>,
    pub strength: f32,
}

/// Edge between notes
#[derive(Debug, Clone)]
pub struct Edge {
    pub from_id: u64,
    pub to_id: u64,
    pub relation: String,
    pub weight: f32,
}

/// Repair result
#[derive(Debug, Clone)]
pub struct RepairResult {
    pub integrity_ok: bool,
    pub errors_fixed: usize,
}

/// Backfill result
#[derive(Debug, Clone)]
pub struct BackfillResult {
    pub embeddings_created: usize,
    pub temporal_links_created: usize,
    pub semantic_links_created: usize,
}

/// Statistics
#[derive(Debug, Clone)]
pub struct NotebookStats {
    pub note_count: u64,
    pub pinned_count: u64,
    pub edge_count: u64,
    pub tag_count: u64,
    pub vault_entries: u64,
    pub embedding_count: u64,
    pub total_content_size: u64,
}

/// Compatibility wrapper around Engram
pub struct NotebookStorage {
    engram: Engram,
    ai_id: String,
}

impl NotebookStorage {
    /// Open or create a notebook
    pub fn open(path: impl AsRef<Path>, ai_id: &str) -> Result<Self> {
        let engram = Engram::open(path)?;
        Ok(Self {
            engram,
            ai_id: ai_id.to_string(),
        })
    }

    /// Get AI ID
    pub fn get_ai_id(&self) -> &str {
        &self.ai_id
    }

    // ═══════════════════════════════════════════════════════════════════
    // CORE NOTE OPERATIONS
    // ═══════════════════════════════════════════════════════════════════

    /// Store a note
    pub fn remember(&mut self, note: &Note) -> Result<u64> {
        let tags: Vec<&str> = note.tags.iter().map(|s| s.as_str()).collect();
        let id = self.engram.remember(&note.content, &tags)?;
        Ok(id)
    }

    /// Search for notes
    pub fn recall(&mut self, query: Option<&str>, limit: i64, _include_content: bool) -> Result<Vec<RecallResult>> {
        let query_str = query.unwrap_or("");
        let results = self.engram.recall_by_keyword(query_str, limit as usize)?;
        Ok(results.into_iter().map(|r| RecallResult {
            note: Note::from(r.note),
            score: r.final_score,
            final_score: r.final_score,
            semantic_score: r.vector_score,
            keyword_score: r.keyword_score,
            graph_score: r.graph_score,
        }).collect())
    }

    /// List recent notes
    pub fn list_notes(&mut self, limit: i64) -> Result<Vec<Note>> {
        let notes = self.engram.list(limit as usize)?;
        Ok(notes.into_iter().map(Note::from).collect())
    }

    /// Get a single note
    pub fn get_note(&mut self, id: i64) -> Result<Option<Note>> {
        let note = self.engram.get(id as u64)?;
        Ok(note.map(Note::from))
    }

    /// Pin a note
    pub fn pin_note(&mut self, id: i64) -> Result<bool> {
        self.engram.pin(id as u64)?;
        Ok(true)
    }

    /// Unpin a note
    pub fn unpin_note(&mut self, id: i64) -> Result<bool> {
        self.engram.unpin(id as u64)?;
        Ok(true)
    }

    /// Update a note (not fully supported - deletes and re-adds)
    pub fn update_note(&mut self, id: i64, content: &str, tags: Option<Vec<String>>) -> Result<bool> {
        // Get existing note for tags if not provided
        let existing = self.engram.get(id as u64)?;
        let tags = tags.unwrap_or_else(|| existing.map(|n| n.tags).unwrap_or_default());

        // Delete old
        self.engram.forget(id as u64)?;

        // Add new with same content
        let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
        self.engram.remember(content, &tag_refs)?;

        Ok(true)
    }

    /// Delete a note
    pub fn delete_note(&mut self, id: i64) -> Result<bool> {
        self.engram.forget(id as u64)?;
        Ok(true)
    }

    /// Get statistics
    pub fn get_stats(&mut self) -> Result<NotebookStats> {
        let stats = self.engram.stats();
        Ok(NotebookStats {
            note_count: stats.note_count,
            pinned_count: stats.pinned_count,
            edge_count: stats.edge_count,
            tag_count: stats.tag_count,
            vault_entries: stats.vault_entries,
            embedding_count: stats.vector_count,
            total_content_size: stats.file_size,
        })
    }

    /// Get pinned notes
    pub fn get_pinned_notes(&mut self, _limit: i64) -> Result<Vec<Note>> {
        let notes = self.engram.pinned()?;
        Ok(notes.into_iter().map(Note::from).collect())
    }

    /// Add tags to a note
    pub fn add_tags(&mut self, id: i64, tags: &[String]) -> Result<bool> {
        // Get existing note
        let note = self.engram.get(id as u64)?;
        if let Some(n) = note {
            let mut all_tags = n.tags.clone();
            for tag in tags {
                if !all_tags.contains(tag) {
                    all_tags.push(tag.clone());
                }
            }
            // Update by re-adding
            self.update_note(id, &n.content, Some(all_tags))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // VAULT OPERATIONS
    // ═══════════════════════════════════════════════════════════════════

    /// Store in vault
    pub fn vault_store(&mut self, key: &str, value: &str) -> Result<()> {
        self.engram.vault_set_string(key, value)?;
        Ok(())
    }

    /// Retrieve from vault
    pub fn vault_retrieve(&self, key: &str) -> Result<Option<String>> {
        Ok(self.engram.vault_get_string(key)?)
    }

    /// List vault keys
    pub fn vault_list(&self) -> Result<Vec<String>> {
        Ok(self.engram.vault_keys())
    }

    // ═══════════════════════════════════════════════════════════════════
    // GRAPH/EDGE OPERATIONS
    // ═══════════════════════════════════════════════════════════════════

    /// Link two notes
    pub fn link_notes(&mut self, from_id: i64, to_id: i64, _relation: &str, weight: f32) -> Result<bool> {
        self.engram.add_semantic_edge(from_id as u64, to_id as u64, weight);
        Ok(true)
    }

    /// Unlink two notes - FAIL LOUDLY: Engram edges are permanent
    pub fn unlink_notes(&mut self, from_id: i64, to_id: i64) -> Result<bool> {
        // FAIL LOUDLY: Edge deletion not implemented in Engram
        Err(anyhow!("unlink_notes not implemented. Engram edges are permanent. Tried to unlink: {} -> {}", from_id, to_id))
    }

    /// Get linked notes - returns (Note, relation, weight) tuples
    pub fn get_linked_notes(&mut self, id: i64, _depth: i64) -> Result<Vec<(Note, String, f32)>> {
        let related = self.engram.get_related(id as u64);
        let mut results = Vec::new();
        for (note_id, weight, edge_type) in related {
            if let Some(note) = self.engram.get(note_id)? {
                // Convert EdgeType enum to string representation
                let relation = format!("{:?}", edge_type);
                results.push((Note::from(note), relation, weight));
            }
        }
        Ok(results)
    }

    /// Get all edges
    pub fn get_all_edges(&mut self, _limit: i64) -> Result<Vec<Edge>> {
        // Not directly supported - return empty
        Ok(Vec::new())
    }

    /// Auto-link temporal
    pub fn auto_link_temporal(&mut self, note_id: i64, window_minutes: i32) -> Result<usize> {
        Ok(self.engram.auto_link_temporal(note_id as u64, window_minutes as i64)?)
    }

    /// Auto-link semantic
    pub fn auto_link_semantic(&mut self, note_id: i64, top_k: usize, min_similarity: f32) -> Result<usize> {
        Ok(self.engram.auto_link_semantic(note_id as u64, min_similarity, top_k)?)
    }

    /// Update PageRank
    pub fn update_pagerank(&mut self) -> Result<()> {
        self.engram.compute_pagerank();
        Ok(())
    }

    /// Backfill embeddings (stub - would need embedding model)
    /// Returns (added, skipped) tuple
    pub fn backfill_embeddings(&mut self) -> Result<(usize, usize)> {
        // FAIL LOUDLY: Embedding generation requires embedding model not available in this context
        Err(anyhow!("backfill_embeddings not implemented. Requires embedding model. Use CLI: notebook-cli.exe backfill"))
    }

    // ═══════════════════════════════════════════════════════════════════
    // SESSION/MAINTENANCE
    // ═══════════════════════════════════════════════════════════════════

    /// Get or create session (stub)
    pub fn get_or_create_session(&self) -> Result<String> {
        // FAIL LOUDLY: Session management not implemented in Engram
        Err(anyhow!("get_or_create_session not implemented. Session tracking is not available in Engram storage."))
    }

    /// Repair
    pub fn repair(&mut self) -> Result<RepairResult> {
        let result = self.engram.verify()?;
        Ok(RepairResult {
            integrity_ok: result.is_valid,
            errors_fixed: result.errors.len(),
        })
    }

    /// Backfill all
    pub fn backfill_all(&mut self) -> Result<BackfillResult> {
        // FAIL LOUDLY: Full backfill requires embedding model not available here
        Err(anyhow!("backfill_all not implemented. Requires embedding model. Use CLI: notebook-cli.exe backfill"))
    }

    // ═══════════════════════════════════════════════════════════════════
    // ENTITY OPERATIONS (STUBS - NOT IMPLEMENTED)
    // ═══════════════════════════════════════════════════════════════════

    pub fn create_entity(&mut self, name: &str, entity_type: &str, _properties: &str, _aliases: Vec<String>, _confidence: f64) -> Result<String> {
        Err(anyhow!("Entity management not implemented - use separate entity store. Tried to create: {} ({})", name, entity_type))
    }

    pub fn find_entity(&mut self, name: &str, _entity_type: Option<&str>) -> Result<Option<Entity>> {
        Err(anyhow!("Entity management not implemented. Tried to find: {}", name))
    }

    pub fn list_entities(&mut self, _entity_type: Option<&str>, _limit: i64, _sort_by: &str) -> Result<Vec<Entity>> {
        // FAIL LOUDLY: Entity management not implemented
        Err(anyhow!("list_entities not implemented. Entity management is not available in Engram."))
    }

    pub fn update_entity(&mut self, entity_id: &str, _field: &str, _value: &str, _rationale: &str, _confidence: f64) -> Result<Option<String>> {
        // FAIL LOUDLY: Entity management not implemented
        Err(anyhow!("update_entity not implemented. Entity management is not available. Tried to update: {}", entity_id))
    }

    pub fn update_relationship(&mut self, from_id: &str, to_id: &str, _relation: &str, _strength: f64, _evidence: Vec<String>) -> Result<(String, bool)> {
        // FAIL LOUDLY: Entity relationships not implemented
        Err(anyhow!("update_relationship not implemented. Entity relationships not available. Tried: {} -> {}", from_id, to_id))
    }

    pub fn get_related_entities(&mut self, entity_id: &str, _relation: Option<&str>, _min_strength: f64) -> Result<Vec<(Entity, String, f32)>> {
        Err(anyhow!("Entity relationships not implemented. Tried to get related to: {}", entity_id))
    }

    pub fn get_entity_updates(&mut self, _entity_id: Option<&str>, _limit: i64) -> Result<Vec<String>> {
        // FAIL LOUDLY: Entity update history not implemented
        Err(anyhow!("get_entity_updates not implemented. Entity management is not available in Engram."))
    }

    pub fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<Entity>> {
        Err(anyhow!("Entity management not implemented. Tried to get: {}", entity_id))
    }

    // ═══════════════════════════════════════════════════════════════════
    // COGNITIVE OPERATIONS (STUBS - NOT IMPLEMENTED)
    // ═══════════════════════════════════════════════════════════════════

    pub fn add_strategy(&mut self, title: &str, _context: &str, _approach: &str, _tags: &[String]) -> Result<String> {
        // Store as a regular note with strategy tag
        let content = format!("STRATEGY: {}", title);
        let tags: Vec<&str> = vec!["strategy"];
        let id = self.engram.remember(&content, &tags)?;
        Ok(id.to_string())
    }

    pub fn add_insight(&mut self, discovery: &str, _evidence: &[String], _confidence: f64, _tags: &[String]) -> Result<String> {
        let content = format!("INSIGHT: {}", discovery);
        let tags: Vec<&str> = vec!["insight"];
        let id = self.engram.remember(&content, &tags)?;
        Ok(id.to_string())
    }

    pub fn add_pattern(&mut self, situation: &str, pattern: &str, _examples: &[String], _strength: f64, _tags: &[String]) -> Result<String> {
        let content = format!("PATTERN: {} -> {}", situation, pattern);
        let tags: Vec<&str> = vec!["pattern"];
        let id = self.engram.remember(&content, &tags)?;
        Ok(id.to_string())
    }

    pub fn strategy_feedback(&mut self, strategy_id: &str, _helpful: bool) -> Result<bool> {
        // FAIL LOUDLY: Strategy feedback tracking not implemented
        Err(anyhow!("strategy_feedback not implemented. Feedback tracking not available. Strategy: {}", strategy_id))
    }

    /// List strategies - returns tuples (id, title, context, effectiveness, approach)
    pub fn list_strategies(&mut self, limit: i64) -> Result<Vec<(String, String, String, f32, String)>> {
        // Search for notes tagged with "strategy"
        let notes = self.engram.by_tag("strategy")?;
        Ok(notes.into_iter().take(limit as usize).map(|n| (
            n.id.to_string(),
            n.content.replace("STRATEGY: ", ""),
            String::new(), // context
            0.5_f32,       // effectiveness
            String::new(), // approach
        )).collect())
    }

    /// List insights - returns tuples (id, discovery, confidence)
    pub fn list_insights(&mut self, limit: i64) -> Result<Vec<(String, String, f32)>> {
        let notes = self.engram.by_tag("insight")?;
        Ok(notes.into_iter().take(limit as usize).map(|n| (
            n.id.to_string(),
            n.content.replace("INSIGHT: ", ""),
            0.5_f32,
        )).collect())
    }

    /// List patterns - returns tuples (id, situation, pattern, strength)
    pub fn list_patterns(&mut self, limit: i64) -> Result<Vec<(String, String, String, f32)>> {
        let notes = self.engram.by_tag("pattern")?;
        Ok(notes.into_iter().take(limit as usize).map(|n| (
            n.id.to_string(),
            n.content.replace("PATTERN: ", ""),
            String::new(),
            0.5_f32,
        )).collect())
    }

    // ═══════════════════════════════════════════════════════════════════
    // TEMPORAL & GRAPH QUERIES (Phase 2 - CLI parity)
    // ═══════════════════════════════════════════════════════════════════

    /// Timeline - get notes sorted by time
    pub fn timeline(&mut self, limit: usize, oldest_first: bool) -> Result<Vec<Note>> {
        let mut notes = self.engram.recent(limit * 2)?;
        if oldest_first {
            notes.sort_by_key(|n| n.timestamp);
        } else {
            notes.sort_by_key(|n| std::cmp::Reverse(n.timestamp));
        }
        Ok(notes.into_iter().take(limit).map(Note::from).collect())
    }

    /// Time range - get notes created in a specific time range
    pub fn time_range(&mut self, start_nanos: i64, end_nanos: i64, limit: usize) -> Result<Vec<Note>> {
        let notes = self.engram.temporal_range(start_nanos, end_nanos)?;
        Ok(notes.into_iter().take(limit).map(Note::from).collect())
    }

    /// Traverse - multi-hop graph traversal from a starting note
    pub fn traverse(&mut self, note_id: u64, max_depth: usize) -> Result<Vec<(Note, String, f32, usize)>> {
        use std::collections::{HashSet, VecDeque};

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut results = Vec::new();

        visited.insert(note_id);
        queue.push_back((note_id, 0usize));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            let neighbors = self.engram.get_related(current_id);
            for (neighbor_id, weight, edge_type) in neighbors {
                if !visited.contains(&neighbor_id) {
                    visited.insert(neighbor_id);
                    queue.push_back((neighbor_id, depth + 1));

                    if let Ok(Some(note)) = self.engram.get(neighbor_id) {
                        let type_str = format!("{:?}", edge_type).to_lowercase();
                        results.push((Note::from(note), type_str, weight, depth + 1));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Find path between two notes in the knowledge graph
    pub fn find_path(&mut self, from_id: u64, to_id: u64, max_depth: usize) -> Result<Vec<(Note, String, f32)>> {
        use std::collections::{HashMap, VecDeque};
        use engram::graph::EdgeType;

        let mut visited: HashMap<u64, (u64, EdgeType, f32)> = HashMap::new();
        let mut queue = VecDeque::new();
        visited.insert(from_id, (0, EdgeType::Manual, 0.0));
        queue.push_back((from_id, 0usize));

        let mut found = false;

        while let Some((current_id, depth)) = queue.pop_front() {
            if current_id == to_id {
                found = true;
                break;
            }

            if depth >= max_depth {
                continue;
            }

            let neighbors = self.engram.get_related(current_id);
            for (neighbor_id, weight, etype) in neighbors {
                if !visited.contains_key(&neighbor_id) {
                    visited.insert(neighbor_id, (current_id, etype, weight));
                    queue.push_back((neighbor_id, depth + 1));
                }
            }
        }

        if !found {
            return Ok(Vec::new());
        }

        // Reconstruct path
        let mut path = Vec::new();
        let mut current = to_id;
        while current != from_id && current != 0 {
            if let Some(&(parent, etype, weight)) = visited.get(&current) {
                if let Ok(Some(note)) = self.engram.get(current) {
                    let type_str = format!("{:?}", etype).to_lowercase();
                    path.push((Note::from(note), type_str, weight));
                }
                current = parent;
            } else {
                break;
            }
        }
        // Add start note
        if let Ok(Some(note)) = self.engram.get(from_id) {
            path.push((Note::from(note), "start".to_string(), 0.0));
        }
        path.reverse();
        Ok(path)
    }

    /// Get related notes with edge types
    pub fn get_related_with_types(&mut self, note_id: u64) -> Result<Vec<(Note, String, f32)>> {
        let related = self.engram.get_related(note_id);
        let mut results = Vec::new();
        for (id, weight, edge_type) in related {
            if let Ok(Some(note)) = self.engram.get(id) {
                let type_str = format!("{:?}", edge_type).to_lowercase();
                results.push((Note::from(note), type_str, weight));
            }
        }
        Ok(results)
    }

    /// Get PageRank for a note
    pub fn get_pagerank(&self, note_id: u64) -> f32 {
        self.engram.get_pagerank(note_id)
    }

    /// Get top notes by PageRank
    pub fn top_notes(&mut self, limit: usize) -> Result<Vec<(Note, f32)>> {
        let notes = self.engram.recent(limit * 5)?;
        let mut with_rank: Vec<_> = notes.into_iter()
            .map(|n| {
                let rank = self.engram.get_pagerank(n.id);
                (Note::from(n), rank)
            })
            .collect();
        with_rank.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(with_rank.into_iter().take(limit).collect())
    }

    /// Health check - verify integrity with suggestions
    pub fn health_check(&mut self) -> Result<(bool, Vec<String>, Vec<String>)> {
        let mut issues = Vec::new();
        let mut warnings = Vec::new();

        let verify_result = self.engram.verify()?;
        if !verify_result.is_valid {
            for err in verify_result.errors {
                issues.push(format!("INTEGRITY: {}", err));
            }
        }
        for warn in verify_result.warnings {
            warnings.push(warn);
        }

        let stats = self.engram.stats();

        // Check embedding coverage
        if stats.active_notes > 0 {
            let coverage = stats.vector_count as f64 / stats.active_notes as f64 * 100.0;
            if coverage < 50.0 {
                warnings.push(format!("LOW EMBEDDING COVERAGE: {:.1}% - Run backfill to generate embeddings", coverage));
            }
        }

        // Check graph connectivity
        if stats.edge_count == 0 && stats.active_notes > 5 {
            warnings.push("NO GRAPH EDGES: Knowledge graph is empty. Run auto-link to create connections".to_string());
        }

        Ok((issues.is_empty(), issues, warnings))
    }

    /// Export notes to JSON
    pub fn export(&mut self, pinned_only: bool, limit: usize) -> Result<String> {
        let notes = if pinned_only {
            self.engram.pinned()?
        } else {
            self.engram.recent(limit)?
        };

        let export_data: Vec<_> = notes.iter().map(|n| {
            serde_json::json!({
                "id": n.id,
                "content": n.content,
                "tags": n.tags,
                "pinned": n.pinned,
                "pagerank": n.pagerank,
                "timestamp": n.timestamp
            })
        }).collect();

        Ok(serde_json::to_string_pretty(&export_data)?)
    }

    /// Has embedding for a note
    pub fn has_embedding(&self, note_id: u64) -> bool {
        self.engram.has_embedding(note_id)
    }

    /// Add embedding for a note
    pub fn add_embedding(&mut self, note_id: u64, embedding: &[f32]) -> Result<()> {
        self.engram.add_embedding(note_id, embedding)?;
        Ok(())
    }
}
