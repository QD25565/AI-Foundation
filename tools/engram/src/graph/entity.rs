//! Entity Extraction and Auto-linking for Engram Knowledge Graph 2.0
//!
//! Provides lightweight entity extraction without LLM dependency:
//! - Regex-based entity detection (capitalized phrases, technical terms)
//! - Entity index for fast lookup
//! - Auto-linking between notes that share entities
//! - Entity clustering to reduce sparsity
//!
//! Designed to work with the note content to automatically build
//! knowledge graph edges based on shared entities.

use std::collections::{HashMap, HashSet};
use regex::Regex;
use super::types::{Edge, EdgeType, SemanticEdge};

/// Maximum input length for entity extraction (64 KiB).
///
/// Text longer than this is truncated at a valid UTF-8 character boundary
/// before any pattern matching. This bounds:
/// - 12 sequential regex `find_iter` scans across the full text
/// - `full_text.matches(entity).count()` called once per extracted entity (O(n×m))
/// - `full_text.contains(indicator)` called ×10 per technical/code entity
///
/// Notes longer than 64 KiB are rare in practice; entities meaningful for
/// graph linking almost always appear early in the content.
const MAX_EXTRACT_BYTES: usize = 64 * 1024;

// ============================================================================
// Entity Types
// ============================================================================

/// Types of entities we can extract
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityType {
    /// Proper nouns (capitalized phrases)
    ProperNoun,
    /// Technical terms (camelCase, snake_case, ACRONYMS)
    TechnicalTerm,
    /// Code identifiers (function names, variables)
    CodeIdentifier,
    /// File paths
    FilePath,
    /// URLs
    Url,
    /// Version numbers
    Version,
    /// Hashtags/Tags
    Tag,
    /// @mentions
    Mention,
    /// Quoted strings
    QuotedString,
}

/// An extracted entity
#[derive(Debug, Clone)]
pub struct Entity {
    /// The entity text (normalized)
    pub text: String,
    /// Original text as found
    pub original: String,
    /// Type of entity
    pub entity_type: EntityType,
    /// Position in source text (start, end)
    pub span: (usize, usize),
    /// Confidence in extraction (0.0 - 1.0)
    pub confidence: f32,
}

impl Entity {
    /// Create a new entity
    pub fn new(text: String, original: String, entity_type: EntityType, span: (usize, usize)) -> Self {
        Self {
            text,
            original,
            entity_type,
            span,
            confidence: 1.0,
        }
    }

    /// Normalize entity text for matching
    pub fn normalize(text: &str) -> String {
        text.trim()
            .to_lowercase()
            .replace(['_', '-'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ============================================================================
// Entity Extractor
// ============================================================================

/// Regex-based entity extractor
pub struct EntityExtractor {
    /// Compiled regex patterns
    patterns: Vec<(EntityType, Regex, f32)>,

    /// Minimum entity length
    min_length: usize,

    /// Maximum entity length
    max_length: usize,

    /// Stop words to filter out
    stop_words: HashSet<String>,
}

impl EntityExtractor {
    /// Create a new entity extractor with default patterns
    pub fn new() -> Self {
        let patterns = vec![
            // URLs (high confidence)
            (EntityType::Url,
             Regex::new(r"https?://[^\s\]\)]+").unwrap(),
             0.95),

            // File paths (high confidence)
            (EntityType::FilePath,
             Regex::new(r"(?:[A-Za-z]:)?(?:/|\\)[\w\-./\\]+\.\w+").unwrap(),
             0.9),

            // @mentions
            (EntityType::Mention,
             Regex::new(r"@[\w\-]+").unwrap(),
             0.95),

            // Hashtags
            (EntityType::Tag,
             Regex::new(r"#[\w\-]+").unwrap(),
             0.95),

            // Version numbers (e.g., v1.2.3, 2.0.0)
            (EntityType::Version,
             Regex::new(r"\bv?\d+\.\d+(?:\.\d+)?(?:-[\w.]+)?\b").unwrap(),
             0.85),

            // ACRONYMS (2+ capital letters)
            (EntityType::TechnicalTerm,
             Regex::new(r"\b[A-Z]{2,}(?:\d+)?\b").unwrap(),
             0.8),

            // CamelCase identifiers
            (EntityType::CodeIdentifier,
             Regex::new(r"\b[A-Z][a-z]+(?:[A-Z][a-z]+)+\b").unwrap(),
             0.85),

            // snake_case identifiers
            (EntityType::CodeIdentifier,
             Regex::new(r"\b[a-z]+(?:_[a-z]+)+\b").unwrap(),
             0.8),

            // Capitalized phrases (2-4 words)
            (EntityType::ProperNoun,
             Regex::new(r"\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,3}\b").unwrap(),
             0.7),

            // Single capitalized words (lower confidence, might be sentence start)
            (EntityType::ProperNoun,
             Regex::new(r"\b[A-Z][a-z]{2,}\b").unwrap(),
             0.5),

            // Quoted strings
            (EntityType::QuotedString,
             Regex::new(r#""([^"]{2,50})""#).unwrap(),
             0.75),

            // Single-quoted strings
            (EntityType::QuotedString,
             Regex::new(r"'([^']{2,50})'").unwrap(),
             0.75),
        ];

        // Common stop words that shouldn't be entities
        let stop_words: HashSet<String> = [
            "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for",
            "of", "with", "by", "from", "as", "is", "was", "are", "were", "been",
            "be", "have", "has", "had", "do", "does", "did", "will", "would",
            "could", "should", "may", "might", "must", "shall", "can", "need",
            "this", "that", "these", "those", "it", "its", "i", "you", "he",
            "she", "we", "they", "my", "your", "his", "her", "our", "their",
            "what", "which", "who", "whom", "when", "where", "why", "how",
            "all", "each", "every", "both", "few", "more", "most", "other",
            "some", "such", "no", "not", "only", "same", "so", "than", "too",
            "very", "just", "also", "now", "here", "there", "then", "once",
            // Common sentence starters
            "however", "therefore", "although", "because", "since", "while",
            "after", "before", "during", "until", "unless", "if", "when",
            // Technical but common
            "true", "false", "null", "none", "error", "warning", "info",
            "note", "todo", "fixme", "bug", "feature", "test",
        ].iter().map(|s| s.to_string()).collect();

        Self {
            patterns,
            min_length: 2,
            max_length: 100,
            stop_words,
        }
    }

    /// Extract entities from text
    pub fn extract(&self, text: &str) -> Vec<Entity> {
        // Bound input size before any regex scanning or per-entity confidence
        // adjustments. All three cost centres (12 regex find_iters, the
        // matches().count() O(n×m) call, and the contains() indicator loop)
        // operate on this slice, so a single guard here covers everything.
        let text = if text.len() > MAX_EXTRACT_BYTES {
            let mut boundary = MAX_EXTRACT_BYTES;
            while boundary > 0 && !text.is_char_boundary(boundary) {
                boundary -= 1;
            }
            &text[..boundary]
        } else {
            text
        };

        let mut entities = Vec::new();
        let mut seen_spans: HashSet<(usize, usize)> = HashSet::new();

        for (entity_type, pattern, base_confidence) in &self.patterns {
            for mat in pattern.find_iter(text) {
                let span = (mat.start(), mat.end());

                // Skip if we've already extracted an entity at this span
                if seen_spans.contains(&span) {
                    continue;
                }

                let original = mat.as_str().to_string();
                let normalized = Entity::normalize(&original);

                // Skip if too short/long
                if normalized.len() < self.min_length || normalized.len() > self.max_length {
                    continue;
                }

                // Skip stop words (for proper nouns)
                if *entity_type == EntityType::ProperNoun &&
                   self.stop_words.contains(&normalized) {
                    continue;
                }

                // Adjust confidence based on context
                let confidence = self.adjust_confidence(&original, *entity_type, *base_confidence, text, span);

                if confidence >= 0.3 {
                    seen_spans.insert(span);
                    entities.push(Entity {
                        text: normalized,
                        original,
                        entity_type: *entity_type,
                        span,
                        confidence,
                    });
                }
            }
        }

        // Sort by position
        entities.sort_by_key(|e| e.span.0);

        // Deduplicate overlapping entities (keep higher confidence)
        self.deduplicate_overlapping(entities)
    }

    /// Adjust confidence based on context
    fn adjust_confidence(
        &self,
        text: &str,
        entity_type: EntityType,
        base_confidence: f32,
        full_text: &str,
        span: (usize, usize),
    ) -> f32 {
        let mut confidence = base_confidence;

        // Boost if appears multiple times
        let count = full_text.matches(text).count();
        if count > 1 {
            confidence *= 1.0 + (count as f32 - 1.0) * 0.1;
        }

        // Reduce confidence for single capitalized words at sentence start
        if entity_type == EntityType::ProperNoun && span.0 > 0 {
            let before = &full_text[..span.0];
            if before.ends_with(". ") || before.ends_with(".\n") ||
               before.ends_with("! ") || before.ends_with("? ") {
                confidence *= 0.6; // Likely sentence start, not a proper noun
            }
        }

        // Boost for technical terms in technical context
        if entity_type == EntityType::TechnicalTerm || entity_type == EntityType::CodeIdentifier {
            let tech_indicators = ["function", "class", "struct", "impl", "fn ", "def ",
                                   "const", "let ", "var ", "pub ", "async", "await"];
            for indicator in tech_indicators {
                if full_text.contains(indicator) {
                    confidence *= 1.1;
                    break;
                }
            }
        }

        confidence.min(1.0)
    }

    /// Remove overlapping entities, keeping higher confidence ones
    fn deduplicate_overlapping(&self, mut entities: Vec<Entity>) -> Vec<Entity> {
        if entities.len() <= 1 {
            return entities;
        }

        // Sort by start position, then by length (longer first)
        entities.sort_by(|a, b| {
            a.span.0.cmp(&b.span.0)
                .then_with(|| (b.span.1 - b.span.0).cmp(&(a.span.1 - a.span.0)))
        });

        let mut result = Vec::new();
        let mut last_end = 0;

        for entity in entities {
            if entity.span.0 >= last_end {
                last_end = entity.span.1;
                result.push(entity);
            } else if entity.confidence > result.last().map(|e| e.confidence).unwrap_or(0.0) {
                // Higher confidence entity overlaps - replace
                if let Some(last) = result.last_mut() {
                    if entity.span.0 < last.span.1 {
                        *last = entity.clone();
                        last_end = entity.span.1;
                    }
                }
            }
        }

        result
    }

    /// Extract and group entities by normalized text
    pub fn extract_grouped(&self, text: &str) -> HashMap<String, Vec<Entity>> {
        let entities = self.extract(text);
        let mut grouped: HashMap<String, Vec<Entity>> = HashMap::new();

        for entity in entities {
            grouped.entry(entity.text.clone())
                .or_insert_with(Vec::new)
                .push(entity);
        }

        grouped
    }
}

impl Default for EntityExtractor {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Entity Index
// ============================================================================

/// Index of entities across all notes for fast lookup
#[derive(Debug, Clone)]
pub struct EntityIndex {
    /// Entity text -> list of (note_id, entity)
    entity_to_notes: HashMap<String, Vec<(u64, Entity)>>,

    /// Note ID -> list of entities in that note
    note_to_entities: HashMap<u64, Vec<Entity>>,

    /// Entity type -> set of entity texts
    type_to_entities: HashMap<EntityType, HashSet<String>>,
}

impl EntityIndex {
    /// Create a new empty entity index
    pub fn new() -> Self {
        Self {
            entity_to_notes: HashMap::new(),
            note_to_entities: HashMap::new(),
            type_to_entities: HashMap::new(),
        }
    }

    /// Add entities for a note
    pub fn add_note(&mut self, note_id: u64, entities: Vec<Entity>) {
        for entity in &entities {
            self.entity_to_notes
                .entry(entity.text.clone())
                .or_insert_with(Vec::new)
                .push((note_id, entity.clone()));

            self.type_to_entities
                .entry(entity.entity_type)
                .or_insert_with(HashSet::new)
                .insert(entity.text.clone());
        }

        self.note_to_entities.insert(note_id, entities);
    }

    /// Remove a note from the index
    pub fn remove_note(&mut self, note_id: u64) {
        if let Some(entities) = self.note_to_entities.remove(&note_id) {
            for entity in entities {
                if let Some(notes) = self.entity_to_notes.get_mut(&entity.text) {
                    notes.retain(|(id, _)| *id != note_id);
                    if notes.is_empty() {
                        self.entity_to_notes.remove(&entity.text);
                        // Also remove from type index
                        if let Some(type_set) = self.type_to_entities.get_mut(&entity.entity_type) {
                            type_set.remove(&entity.text);
                        }
                    }
                }
            }
        }
    }

    /// Get all notes containing an entity
    pub fn notes_with_entity(&self, entity_text: &str) -> Vec<u64> {
        let normalized = Entity::normalize(entity_text);
        self.entity_to_notes
            .get(&normalized)
            .map(|notes| notes.iter().map(|(id, _)| *id).collect())
            .unwrap_or_default()
    }

    /// Get all entities in a note
    pub fn entities_in_note(&self, note_id: u64) -> &[Entity] {
        self.note_to_entities
            .get(&note_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get all entities of a specific type
    pub fn entities_of_type(&self, entity_type: EntityType) -> Vec<&str> {
        self.type_to_entities
            .get(&entity_type)
            .map(|set| set.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Find notes that share entities with a given note
    pub fn related_notes(&self, note_id: u64) -> Vec<(u64, usize)> {
        let mut related: HashMap<u64, usize> = HashMap::new();

        if let Some(entities) = self.note_to_entities.get(&note_id) {
            for entity in entities {
                if let Some(notes) = self.entity_to_notes.get(&entity.text) {
                    for (other_id, _) in notes {
                        if *other_id != note_id {
                            *related.entry(*other_id).or_insert(0) += 1;
                        }
                    }
                }
            }
        }

        let mut result: Vec<_> = related.into_iter().collect();
        result.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by shared entity count
        result
    }

    /// Get the most common entities
    pub fn top_entities(&self, limit: usize) -> Vec<(&str, usize)> {
        let mut counts: Vec<_> = self.entity_to_notes
            .iter()
            .map(|(text, notes)| (text.as_str(), notes.len()))
            .collect();

        counts.sort_by(|a, b| b.1.cmp(&a.1));
        counts.truncate(limit);
        counts
    }

    /// Get statistics
    pub fn stats(&self) -> EntityIndexStats {
        EntityIndexStats {
            unique_entities: self.entity_to_notes.len(),
            indexed_notes: self.note_to_entities.len(),
            total_mentions: self.entity_to_notes.values().map(|v| v.len()).sum(),
            entities_by_type: self.type_to_entities
                .iter()
                .map(|(t, s)| (*t, s.len()))
                .collect(),
        }
    }
}

impl Default for EntityIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the entity index
#[derive(Debug, Clone)]
pub struct EntityIndexStats {
    pub unique_entities: usize,
    pub indexed_notes: usize,
    pub total_mentions: usize,
    pub entities_by_type: HashMap<EntityType, usize>,
}

// ============================================================================
// Auto-Linker
// ============================================================================

/// Automatically creates edges between notes based on shared entities
pub struct AutoLinker {
    /// Entity extractor
    extractor: EntityExtractor,

    /// Entity index
    index: EntityIndex,

    /// Minimum shared entities to create a link
    min_shared_entities: usize,

    /// Minimum confidence for entity to count
    min_entity_confidence: f32,
}

impl AutoLinker {
    /// Create a new auto-linker
    pub fn new() -> Self {
        Self {
            extractor: EntityExtractor::new(),
            index: EntityIndex::new(),
            min_shared_entities: 1,
            min_entity_confidence: 0.5,
        }
    }

    /// Create with custom thresholds
    pub fn with_thresholds(min_shared: usize, min_confidence: f32) -> Self {
        Self {
            extractor: EntityExtractor::new(),
            index: EntityIndex::new(),
            min_shared_entities: min_shared,
            min_entity_confidence: min_confidence,
        }
    }

    /// Process a new note and return edges to create
    pub fn process_note(&mut self, note_id: u64, content: &str) -> Vec<Edge> {
        // Extract entities
        let entities: Vec<Entity> = self.extractor.extract(content)
            .into_iter()
            .filter(|e| e.confidence >= self.min_entity_confidence)
            .collect();

        // Accumulate shared entity count and best weight per related note.
        // An edge is only emitted when the shared count reaches min_shared_entities,
        // making with_thresholds() truthful. Default threshold is 1, so existing
        // behaviour (any single shared entity → edge) is unchanged.
        let mut shared_counts: HashMap<u64, usize> = HashMap::new();
        let mut best_weights: HashMap<u64, f32> = HashMap::new();

        for entity in &entities {
            let related_notes = self.index.notes_with_entity(&entity.text);
            for related_id in related_notes {
                if related_id != note_id {
                    *shared_counts.entry(related_id).or_insert(0) += 1;
                    let weight = self.calculate_link_weight(note_id, related_id, &entity.text);
                    let best = best_weights.entry(related_id).or_insert(0.0_f32);
                    if weight > *best {
                        *best = weight;
                    }
                }
            }
        }

        // Add note to index before building the edge list
        self.index.add_note(note_id, entities);

        // Emit one edge per qualifying related note (already one-per-pair, no dedup needed)
        shared_counts
            .into_iter()
            .filter(|(_, count)| *count >= self.min_shared_entities)
            .map(|(related_id, _)| {
                Edge::new(
                    note_id,
                    related_id,
                    EdgeType::Semantic(SemanticEdge::RelatedTo),
                    best_weights[&related_id],
                )
            })
            .collect()
    }

    /// Calculate link weight based on shared entity importance
    fn calculate_link_weight(&self, _note_id: u64, _related_id: u64, entity_text: &str) -> f32 {
        // Weight based on entity rarity (rarer = stronger link)
        let mention_count = self.index.notes_with_entity(entity_text).len();

        if mention_count <= 1 {
            0.9 // Very rare entity = strong link
        } else if mention_count <= 5 {
            0.7
        } else if mention_count <= 20 {
            0.5
        } else {
            0.3 // Very common entity = weak link
        }
    }

    /// Remove a note from the index
    pub fn remove_note(&mut self, note_id: u64) {
        self.index.remove_note(note_id);
    }

    /// Get the entity index
    pub fn index(&self) -> &EntityIndex {
        &self.index
    }

    /// Get mutable entity index
    pub fn index_mut(&mut self) -> &mut EntityIndex {
        &mut self.index
    }

    /// Find all edges that should exist based on current index
    pub fn generate_all_edges(&self) -> Vec<Edge> {
        let mut edges = Vec::new();
        let mut seen_pairs: HashSet<(u64, u64)> = HashSet::new();

        for (entity_text, notes) in &self.index.entity_to_notes {
            if notes.len() < 2 {
                continue;
            }

            // Create edges between all notes sharing this entity
            for i in 0..notes.len() {
                for j in (i + 1)..notes.len() {
                    let (id1, _) = &notes[i];
                    let (id2, _) = &notes[j];

                    let key = ((*id1).min(*id2), (*id1).max(*id2));
                    if seen_pairs.contains(&key) {
                        continue;
                    }
                    seen_pairs.insert(key);

                    let weight = self.calculate_link_weight(*id1, *id2, entity_text);
                    edges.push(Edge::new(
                        *id1,
                        *id2,
                        EdgeType::Semantic(SemanticEdge::RelatedTo),
                        weight,
                    ));
                }
            }
        }

        edges
    }
}

impl Default for AutoLinker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_urls() {
        let extractor = EntityExtractor::new();
        let text = "Check out https://github.com/example/repo for more info.";
        let entities = extractor.extract(text);

        assert!(entities.iter().any(|e| e.entity_type == EntityType::Url));
        assert!(entities.iter().any(|e| e.text.contains("github.com")));
    }

    #[test]
    fn test_extract_mentions() {
        let extractor = EntityExtractor::new();
        let text = "Hey @john-doe, can you review this?";
        let entities = extractor.extract(text);

        assert!(entities.iter().any(|e|
            e.entity_type == EntityType::Mention && e.original == "@john-doe"
        ));
    }

    #[test]
    fn test_extract_tags() {
        let extractor = EntityExtractor::new();
        let text = "This is about #rust and #performance";
        let entities = extractor.extract(text);

        let tags: Vec<_> = entities.iter()
            .filter(|e| e.entity_type == EntityType::Tag)
            .collect();
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn test_extract_camel_case() {
        let extractor = EntityExtractor::new();
        let text = "The EntityExtractor class handles CamelCase identifiers.";
        let entities = extractor.extract(text);

        assert!(entities.iter().any(|e|
            e.entity_type == EntityType::CodeIdentifier && e.original == "EntityExtractor"
        ));
        assert!(entities.iter().any(|e|
            e.entity_type == EntityType::CodeIdentifier && e.original == "CamelCase"
        ));
    }

    #[test]
    fn test_extract_snake_case() {
        let extractor = EntityExtractor::new();
        let text = "The entity_extractor function uses snake_case naming.";
        let entities = extractor.extract(text);

        assert!(entities.iter().any(|e|
            e.entity_type == EntityType::CodeIdentifier && e.original == "entity_extractor"
        ));
    }

    #[test]
    fn test_extract_acronyms() {
        let extractor = EntityExtractor::new();
        let text = "The API uses REST and JSON for HTTP requests.";
        let entities = extractor.extract(text);

        let acronyms: Vec<_> = entities.iter()
            .filter(|e| e.entity_type == EntityType::TechnicalTerm)
            .collect();
        assert!(acronyms.len() >= 3); // API, REST, JSON, HTTP
    }

    #[test]
    fn test_extract_proper_nouns() {
        let extractor = EntityExtractor::new();
        let text = "John Smith works at Acme Corporation in New York.";
        let entities = extractor.extract(text);

        let proper_nouns: Vec<_> = entities.iter()
            .filter(|e| e.entity_type == EntityType::ProperNoun)
            .collect();
        assert!(proper_nouns.len() >= 2);
    }

    #[test]
    fn test_extract_versions() {
        let extractor = EntityExtractor::new();
        let text = "Upgraded from v1.2.3 to version 2.0.0-beta.1";
        let entities = extractor.extract(text);

        let versions: Vec<_> = entities.iter()
            .filter(|e| e.entity_type == EntityType::Version)
            .collect();
        assert_eq!(versions.len(), 2);
    }

    #[test]
    fn test_entity_index() {
        let mut index = EntityIndex::new();
        let extractor = EntityExtractor::new();

        // Use @mentions for reliable entity extraction
        let text1 = "Message from @john-doe about the API.";
        let text2 = "Reply to @john-doe regarding REST.";

        let entities1 = extractor.extract(text1);
        let entities2 = extractor.extract(text2);

        // Verify @john-doe extracted from both
        assert!(entities1.iter().any(|e| e.original == "@john-doe"));
        assert!(entities2.iter().any(|e| e.original == "@john-doe"));

        index.add_note(1, entities1);
        index.add_note(2, entities2);

        // Both notes should share "@john-doe"
        let related = index.related_notes(1);
        assert!(related.iter().any(|(id, _)| *id == 2));
    }

    #[test]
    fn test_auto_linker() {
        let mut linker = AutoLinker::new();

        // Process first note
        let edges1 = linker.process_note(1, "The EntityExtractor handles entity extraction.");
        assert!(edges1.is_empty()); // No related notes yet

        // Process second note with shared entity
        let edges2 = linker.process_note(2, "We use EntityExtractor for NLP tasks.");

        // Should have an edge between notes 1 and 2
        assert!(!edges2.is_empty());
        assert!(edges2.iter().any(|e|
            (e.source == 1 && e.target == 2) || (e.source == 2 && e.target == 1)
        ));
    }

    #[test]
    fn test_stop_words_filtered() {
        let extractor = EntityExtractor::new();
        let text = "The quick brown fox. However, it was fast.";
        let entities = extractor.extract(text);

        // "The" and "However" should be filtered as stop words
        assert!(!entities.iter().any(|e| e.text == "the"));
        assert!(!entities.iter().any(|e| e.text == "however"));
    }

    #[test]
    fn test_entity_normalization() {
        assert_eq!(Entity::normalize("EntityExtractor"), "entityextractor");
        assert_eq!(Entity::normalize("snake_case"), "snake case");
        assert_eq!(Entity::normalize("  Multiple   Spaces  "), "multiple spaces");
    }

    #[test]
    fn test_auto_linker_min_shared_entities_threshold() {
        // with_thresholds(2, …) must require 2 shared entities before emitting an edge.
        let mut linker = AutoLinker::with_thresholds(2, 0.5);

        // Note 1 — two high-confidence entities: @alice and @bob
        linker.process_note(1, "Message from @alice and @bob.");

        // Note 2 shares only @alice (1 shared entity — below threshold of 2)
        let edges_one_shared = linker.process_note(2, "Reply to @alice only.");
        assert!(
            edges_one_shared.is_empty(),
            "one shared entity must not create an edge when threshold is 2"
        );

        // Note 3 shares both @alice and @bob (2 shared entities — meets threshold)
        let edges_two_shared = linker.process_note(3, "Talking to @alice and @bob again.");
        assert!(
            !edges_two_shared.is_empty(),
            "two shared entities must create an edge when threshold is 2"
        );
        assert!(
            edges_two_shared.iter().any(|e| {
                (e.source == 1 && e.target == 3) || (e.source == 3 && e.target == 1)
            }),
            "edge must connect note 1 and note 3"
        );
    }

    #[test]
    fn test_large_input_truncated() {
        let extractor = EntityExtractor::new();

        // Entity at the start, followed by > MAX_EXTRACT_BYTES of filler.
        let prefix = "@alice needs to review this. ";
        // ~100 KiB filler — well beyond the 64 KiB cap.
        let filler = "word ".repeat(25_000);
        let text = format!("{}{}", prefix, filler);
        assert!(
            text.len() > MAX_EXTRACT_BYTES,
            "test input must exceed MAX_EXTRACT_BYTES"
        );

        // Must still extract the entity from within the first 64 KiB.
        let entities = extractor.extract(&text);
        assert!(
            entities.iter().any(|e| e.original == "@alice"),
            "entity before truncation point must be extracted"
        );
    }

    #[test]
    fn test_large_input_entity_beyond_truncation_not_extracted() {
        let extractor = EntityExtractor::new();

        // Push @bob past the 64 KiB boundary.
        let filler = "word ".repeat(15_000); // ~75 KiB
        let text = format!("{}@bob is here", filler);
        assert!(
            text.len() > MAX_EXTRACT_BYTES,
            "test input must exceed MAX_EXTRACT_BYTES"
        );
        assert!(
            filler.len() > MAX_EXTRACT_BYTES,
            "@bob must be beyond the truncation point"
        );

        // @bob is past the 64 KiB boundary — must NOT appear in results.
        let entities = extractor.extract(&text);
        assert!(
            !entities.iter().any(|e| e.original == "@bob"),
            "entity beyond truncation point must not be extracted"
        );
    }

    #[test]
    fn test_index_stats() {
        let mut index = EntityIndex::new();
        let extractor = EntityExtractor::new();

        index.add_note(1, extractor.extract("API endpoint for REST"));
        index.add_note(2, extractor.extract("REST API design patterns"));

        let stats = index.stats();
        assert!(stats.unique_entities > 0);
        assert_eq!(stats.indexed_notes, 2);
    }
}
