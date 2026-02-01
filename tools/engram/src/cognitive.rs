//! Cognitive Memory Types for Engram
//!
//! Classifies notes into memory types based on cognitive science:
//! - **Semantic**: Facts, concepts, definitions, general knowledge
//! - **Episodic**: Events, experiences, stories, specific instances
//! - **Procedural**: How-to instructions, processes, step-by-step guides
//!
//! # Usage
//!
//! ```rust,ignore
//! use engram::cognitive::{MemoryType, classify_content};
//!
//! let content = "To deploy the app: 1. Build 2. Copy to server 3. Restart";
//! let memory_type = classify_content(content);
//! assert_eq!(memory_type, MemoryType::Procedural);
//! ```

use serde::{Deserialize, Serialize};

/// Memory types based on cognitive science classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MemoryType {
    /// Facts, concepts, definitions, general knowledge
    /// Examples: "Rust uses ownership for memory safety", "PostgreSQL is a relational database"
    Semantic = 0,

    /// Events, experiences, stories, specific instances in time
    /// Examples: "Fixed the login bug on Dec 14", "Session summary from yesterday"
    Episodic = 1,

    /// How-to instructions, processes, step-by-step guides
    /// Examples: "To deploy: 1. Build 2. Copy 3. Test", "How to configure OAuth"
    Procedural = 2,
}

impl MemoryType {
    /// Convert from byte
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(MemoryType::Semantic),
            1 => Some(MemoryType::Episodic),
            2 => Some(MemoryType::Procedural),
            _ => None,
        }
    }

    /// Convert to byte
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Get display name
    pub fn name(&self) -> &'static str {
        match self {
            MemoryType::Semantic => "semantic",
            MemoryType::Episodic => "episodic",
            MemoryType::Procedural => "procedural",
        }
    }

    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "semantic" | "fact" | "concept" | "definition" => Some(MemoryType::Semantic),
            "episodic" | "event" | "experience" | "story" => Some(MemoryType::Episodic),
            "procedural" | "howto" | "how-to" | "process" | "steps" => Some(MemoryType::Procedural),
            _ => None,
        }
    }
}

impl Default for MemoryType {
    fn default() -> Self {
        MemoryType::Semantic
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ============================================================================
// Auto-Classification
// ============================================================================

/// Patterns that suggest procedural memory (how-to, instructions)
const PROCEDURAL_PATTERNS: &[&str] = &[
    "how to", "how-to", "howto",
    "step 1", "step 2", "step 3",
    "1.", "2.", "3.",  // Numbered steps
    "first,", "then,", "finally,", "next,",
    "to do this", "in order to",
    "instructions", "guide", "tutorial",
    "run the command", "execute", "deploy",
    "install", "configure", "setup", "set up",
    "follow these", "steps:", "process:",
];

/// Patterns that suggest episodic memory (events, experiences)
const EPISODIC_PATTERNS: &[&str] = &[
    // Date patterns
    "2024-", "2025-", "2026-",
    "today", "yesterday", "last week", "last night",
    "this morning", "this afternoon", "this evening",
    "on monday", "on tuesday", "on wednesday", "on thursday", "on friday",
    "january", "february", "march", "april", "may", "june",
    "july", "august", "september", "october", "november", "december",
    // Event markers
    "session", "complete", "finished", "done", "fixed",
    "discovered", "found", "realized", "learned",
    "happened", "occurred", "experienced",
    "meeting", "discussion", "conversation",
    "milestone", "breakthrough", "success", "failure",
    // Past tense verbs
    "implemented", "deployed", "resolved", "debugged",
    "created", "built", "designed", "tested",
];

/// Classify content into a memory type based on patterns
///
/// Uses keyword matching to determine the most likely memory type.
/// Falls back to Semantic if no strong signals are found.
pub fn classify_content(content: &str) -> MemoryType {
    let lower = content.to_lowercase();

    // Count pattern matches for each type
    let procedural_score = count_pattern_matches(&lower, PROCEDURAL_PATTERNS);
    let episodic_score = count_pattern_matches(&lower, EPISODIC_PATTERNS);

    // Additional heuristics
    let has_numbered_list = has_numbered_steps(&lower);
    let has_date_marker = has_date_pattern(&lower);

    let procedural_total = procedural_score + if has_numbered_list { 3 } else { 0 };
    let episodic_total = episodic_score + if has_date_marker { 2 } else { 0 };

    // Determine type based on scores
    if procedural_total >= 2 && procedural_total > episodic_total {
        MemoryType::Procedural
    } else if episodic_total >= 2 {
        MemoryType::Episodic
    } else {
        MemoryType::Semantic // Default to semantic (facts/concepts)
    }
}

/// Count how many patterns match in the content
fn count_pattern_matches(content: &str, patterns: &[&str]) -> usize {
    patterns.iter().filter(|p| content.contains(*p)).count()
}

/// Check if content has numbered steps (1. 2. 3. pattern)
fn has_numbered_steps(content: &str) -> bool {
    let has_1 = content.contains("1.") || content.contains("1)") || content.contains("step 1");
    let has_2 = content.contains("2.") || content.contains("2)") || content.contains("step 2");
    has_1 && has_2
}

/// Check if content has date patterns
fn has_date_pattern(content: &str) -> bool {
    // Check for ISO date pattern YYYY-MM-DD
    // Use char_indices() for proper Unicode handling - gives (byte_index, char)
    // instead of chars().enumerate() which gives (char_count, char)
    let has_iso_date = content.char_indices().any(|(byte_idx, c)| {
        if c == '-' && byte_idx >= 4 {
            // Safe byte slicing - check that start index is valid
            let start = byte_idx.saturating_sub(4);
            // Verify start is on a char boundary before slicing
            if content.is_char_boundary(start) && content.is_char_boundary(byte_idx) {
                let before = &content[start..byte_idx];
                before.chars().all(|ch| ch.is_ascii_digit())
            } else {
                false
            }
        } else {
            false
        }
    });

    // Check for parenthetical date like (2025-12-14)
    let has_paren_date = content.contains("(2024-") || content.contains("(2025-");

    has_iso_date || has_paren_date
}

/// Suggest query memory type based on query content
///
/// Helps route queries to the right memory type for better recall.
pub fn suggest_query_type(query: &str) -> Option<MemoryType> {
    let lower = query.to_lowercase();

    // Procedural queries
    if lower.starts_with("how to") || lower.starts_with("how do") ||
       lower.contains("steps to") || lower.contains("process for") ||
       lower.contains("instructions") || lower.contains("guide") {
        return Some(MemoryType::Procedural);
    }

    // Episodic queries
    if lower.contains("when did") || lower.contains("what happened") ||
       lower.contains("last time") || lower.contains("session") ||
       lower.contains("yesterday") || lower.contains("today") {
        return Some(MemoryType::Episodic);
    }

    // Semantic queries
    if lower.starts_with("what is") || lower.starts_with("what are") ||
       lower.contains("definition") || lower.contains("explain") ||
       lower.contains("means") || lower.contains("concept") {
        return Some(MemoryType::Semantic);
    }

    None // No strong signal, search all types
}

// ============================================================================
// Memory Type Statistics
// ============================================================================

/// Statistics about memory type distribution
#[derive(Debug, Clone, Default)]
pub struct MemoryTypeStats {
    pub semantic_count: u64,
    pub episodic_count: u64,
    pub procedural_count: u64,
}

impl MemoryTypeStats {
    pub fn total(&self) -> u64 {
        self.semantic_count + self.episodic_count + self.procedural_count
    }

    pub fn increment(&mut self, memory_type: MemoryType) {
        match memory_type {
            MemoryType::Semantic => self.semantic_count += 1,
            MemoryType::Episodic => self.episodic_count += 1,
            MemoryType::Procedural => self.procedural_count += 1,
        }
    }

    pub fn percentage(&self, memory_type: MemoryType) -> f32 {
        let total = self.total() as f32;
        if total == 0.0 {
            return 0.0;
        }
        match memory_type {
            MemoryType::Semantic => self.semantic_count as f32 / total * 100.0,
            MemoryType::Episodic => self.episodic_count as f32 / total * 100.0,
            MemoryType::Procedural => self.procedural_count as f32 / total * 100.0,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_procedural_classification() {
        let content = "How to deploy the app: 1. Build the project 2. Copy files to server 3. Restart services";
        assert_eq!(classify_content(content), MemoryType::Procedural);

        let content2 = "Step 1: Install Rust. Step 2: Run cargo build. Step 3: Execute the binary.";
        assert_eq!(classify_content(content2), MemoryType::Procedural);
    }

    #[test]
    fn test_episodic_classification() {
        let content = "PHASE 1 COMPLETE (2025-12-14): Successfully implemented the feature today.";
        assert_eq!(classify_content(content), MemoryType::Episodic);

        let content2 = "Yesterday we discovered a critical bug and fixed it this morning.";
        assert_eq!(classify_content(content2), MemoryType::Episodic);
    }

    #[test]
    fn test_semantic_classification() {
        let content = "PostgreSQL is a relational database that uses B+ trees for indexing.";
        assert_eq!(classify_content(content), MemoryType::Semantic);

        let content2 = "Rust's ownership system ensures memory safety without garbage collection.";
        assert_eq!(classify_content(content2), MemoryType::Semantic);
    }

    #[test]
    fn test_query_type_suggestion() {
        assert_eq!(suggest_query_type("how to deploy"), Some(MemoryType::Procedural));
        assert_eq!(suggest_query_type("what happened yesterday"), Some(MemoryType::Episodic));
        assert_eq!(suggest_query_type("what is Rust"), Some(MemoryType::Semantic));
        assert_eq!(suggest_query_type("login bug"), None);
    }

    #[test]
    fn test_memory_type_from_str() {
        assert_eq!(MemoryType::from_str("semantic"), Some(MemoryType::Semantic));
        assert_eq!(MemoryType::from_str("EPISODIC"), Some(MemoryType::Episodic));
        assert_eq!(MemoryType::from_str("how-to"), Some(MemoryType::Procedural));
        assert_eq!(MemoryType::from_str("invalid"), None);
    }

    #[test]
    fn test_memory_type_stats() {
        let mut stats = MemoryTypeStats::default();
        stats.increment(MemoryType::Semantic);
        stats.increment(MemoryType::Semantic);
        stats.increment(MemoryType::Episodic);

        assert_eq!(stats.total(), 3);
        assert!((stats.percentage(MemoryType::Semantic) - 66.66).abs() < 1.0);
    }

    #[test]
    fn test_unicode_content_classification() {
        // Test that Unicode/emoji content doesn't crash
        let content_with_emoji = "🚀 Launched feature on 2025-12-14! Great success! 🎉";
        let result = classify_content(content_with_emoji);
        assert_eq!(result, MemoryType::Episodic);

        // More emoji-heavy content
        let emoji_heavy = "💜 Fixed bug 🐛 yesterday - session complete ✅";
        let result2 = classify_content(emoji_heavy);
        assert_eq!(result2, MemoryType::Episodic);

        // Pure emoji with date pattern
        let with_date = "🔥🔥🔥 2025-01-01 New Year! 🎆";
        let result3 = classify_content(with_date);
        // Should not panic, classification may vary
        let _ = result3;
    }

    #[test]
    fn test_has_date_pattern_unicode() {
        // Ensure has_date_pattern doesn't panic on Unicode
        let content = "🎉 Celebration on 2025-12-14 was amazing! 🎊";
        assert!(has_date_pattern(&content.to_lowercase()));

        // Multi-byte chars before the date
        let content2 = "日本語テスト 2025-01-15 test";
        let _ = has_date_pattern(&content2.to_lowercase());
    }
}
