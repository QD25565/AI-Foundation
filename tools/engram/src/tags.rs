//! Tag index for fast tag-based lookups

use std::collections::{HashMap, HashSet};

/// Tag index - maps tags to note IDs
pub struct TagIndex {
    /// Tag -> set of note IDs
    tags: HashMap<String, HashSet<u64>>,

    /// Note ID -> set of tags (reverse index)
    notes: HashMap<u64, HashSet<String>>,
}

impl TagIndex {
    /// Create a new empty tag index
    pub fn new() -> Self {
        Self {
            tags: HashMap::new(),
            notes: HashMap::new(),
        }
    }

    /// Add tags for a note
    pub fn add(&mut self, note_id: u64, tags: &[String]) {
        for tag in tags {
            self.tags
                .entry(tag.clone())
                .or_insert_with(HashSet::new)
                .insert(note_id);

            self.notes
                .entry(note_id)
                .or_insert_with(HashSet::new)
                .insert(tag.clone());
        }
    }

    /// Remove all tags for a note
    pub fn remove_note(&mut self, note_id: u64) {
        if let Some(note_tags) = self.notes.remove(&note_id) {
            for tag in note_tags {
                if let Some(ids) = self.tags.get_mut(&tag) {
                    ids.remove(&note_id);
                    if ids.is_empty() {
                        self.tags.remove(&tag);
                    }
                }
            }
        }
    }

    /// Get all note IDs with a given tag
    pub fn get(&self, tag: &str) -> Vec<u64> {
        self.tags
            .get(tag)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get all tags for a note
    pub fn tags_for_note(&self, note_id: u64) -> Vec<String> {
        self.notes
            .get(&note_id)
            .map(|tags| tags.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Check if a tag exists
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains_key(tag)
    }

    /// Get all unique tags
    pub fn all_tags(&self) -> Vec<String> {
        self.tags.keys().cloned().collect()
    }

    /// Number of unique tags
    pub fn tag_count(&self) -> usize {
        self.tags.len()
    }

    /// Get notes matching ALL given tags (intersection)
    pub fn match_all(&self, tags: &[&str]) -> Vec<u64> {
        if tags.is_empty() {
            return Vec::new();
        }

        let mut result: Option<HashSet<u64>> = None;

        for tag in tags {
            if let Some(ids) = self.tags.get(*tag) {
                result = Some(match result {
                    Some(existing) => existing.intersection(ids).copied().collect(),
                    None => ids.clone(),
                });
            } else {
                // Tag doesn't exist, no matches
                return Vec::new();
            }
        }

        result.map(|s| s.into_iter().collect()).unwrap_or_default()
    }

    /// Get notes matching ANY given tags (union)
    pub fn match_any(&self, tags: &[&str]) -> Vec<u64> {
        let mut result = HashSet::new();

        for tag in tags {
            if let Some(ids) = self.tags.get(*tag) {
                result.extend(ids.iter().copied());
            }
        }

        result.into_iter().collect()
    }
}

impl Default for TagIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get() {
        let mut index = TagIndex::new();

        index.add(1, &["rust".into(), "database".into()]);
        index.add(2, &["rust".into(), "ai".into()]);
        index.add(3, &["python".into()]);

        let rust_notes = index.get("rust");
        assert_eq!(rust_notes.len(), 2);
        assert!(rust_notes.contains(&1));
        assert!(rust_notes.contains(&2));
    }

    #[test]
    fn test_remove_note() {
        let mut index = TagIndex::new();

        index.add(1, &["rust".into()]);
        index.add(2, &["rust".into()]);

        index.remove_note(1);

        let rust_notes = index.get("rust");
        assert_eq!(rust_notes.len(), 1);
        assert!(rust_notes.contains(&2));
    }

    #[test]
    fn test_match_all() {
        let mut index = TagIndex::new();

        index.add(1, &["rust".into(), "database".into()]);
        index.add(2, &["rust".into(), "ai".into()]);
        index.add(3, &["rust".into(), "database".into(), "ai".into()]);

        let matches = index.match_all(&["rust", "database"]);
        assert_eq!(matches.len(), 2);
        assert!(matches.contains(&1));
        assert!(matches.contains(&3));
    }

    #[test]
    fn test_match_any() {
        let mut index = TagIndex::new();

        index.add(1, &["rust".into()]);
        index.add(2, &["python".into()]);
        index.add(3, &["go".into()]);

        let matches = index.match_any(&["rust", "python"]);
        assert_eq!(matches.len(), 2);
        assert!(matches.contains(&1));
        assert!(matches.contains(&2));
    }
}
