//! Temporal index for time-based queries
//!
//! Sorted array enabling efficient range queries by timestamp.

/// Temporal index - sorted (timestamp, note_id) pairs
pub struct TemporalIndex {
    /// Sorted by timestamp ascending
    entries: Vec<(i64, u64)>,
}

impl TemporalIndex {
    /// Create a new empty temporal index
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add an entry
    pub fn add(&mut self, timestamp: i64, note_id: u64) {
        // Binary search for insertion point
        let idx = self.entries
            .binary_search_by_key(&timestamp, |e| e.0)
            .unwrap_or_else(|i| i);

        self.entries.insert(idx, (timestamp, note_id));
    }

    /// Remove an entry by note ID
    pub fn remove(&mut self, note_id: u64) {
        self.entries.retain(|(_, id)| *id != note_id);
    }

    /// Query notes in a time range [start, end]
    pub fn range(&self, start: i64, end: i64) -> impl Iterator<Item = u64> + '_ {
        // Find start index
        let start_idx = self.entries
            .binary_search_by_key(&start, |e| e.0)
            .unwrap_or_else(|i| i);

        // Find end index - use saturating_add to prevent overflow at i64::MAX
        let end_idx = self.entries
            .binary_search_by_key(&end.saturating_add(1), |e| e.0)
            .unwrap_or_else(|i| i);

        self.entries[start_idx..end_idx].iter().map(|(_, id)| *id)
    }

    /// Get most recent N note IDs
    pub fn recent(&self, limit: usize) -> impl Iterator<Item = u64> + '_ {
        self.entries.iter().rev().take(limit).map(|(_, id)| *id)
    }

    /// Get oldest N note IDs
    pub fn oldest(&self, limit: usize) -> impl Iterator<Item = u64> + '_ {
        self.entries.iter().take(limit).map(|(_, id)| *id)
    }

    /// Number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is empty?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get notes created within the last N seconds
    pub fn within_seconds(&self, seconds: i64) -> impl Iterator<Item = u64> + '_ {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let cutoff = now - (seconds * 1_000_000_000);
        self.range(cutoff, now)
    }
}

impl Default for TemporalIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_range() {
        let mut index = TemporalIndex::new();

        index.add(100, 1);
        index.add(200, 2);
        index.add(300, 3);
        index.add(150, 4); // Out of order

        let range: Vec<u64> = index.range(100, 200).collect();
        assert_eq!(range, vec![1, 4, 2]);
    }

    #[test]
    fn test_recent() {
        let mut index = TemporalIndex::new();

        for i in 1..=10 {
            index.add(i * 100, i as u64);
        }

        let recent: Vec<u64> = index.recent(3).collect();
        assert_eq!(recent, vec![10, 9, 8]);
    }

    #[test]
    fn test_remove() {
        let mut index = TemporalIndex::new();

        index.add(100, 1);
        index.add(200, 2);
        index.add(300, 3);

        index.remove(2);

        assert_eq!(index.len(), 2);
        let all: Vec<u64> = index.range(0, i64::MAX).collect();
        assert_eq!(all, vec![1, 3]);
    }
}
