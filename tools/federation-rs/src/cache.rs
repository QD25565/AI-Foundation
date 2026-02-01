//! SharedCache - Distributed cache for the federation mesh

use crate::{DataCategory, Result, FederationError};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// A cache entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The cached data
    pub data: Vec<u8>,

    /// Data category
    pub category: DataCategory,

    /// Key for this entry
    pub key: String,

    /// Which node originated this data
    pub origin_node: String,

    /// Version number (for conflict resolution)
    pub version: u64,

    /// When this entry was created
    pub created_at: DateTime<Utc>,

    /// When this entry expires
    pub expires_at: Option<DateTime<Utc>>,

    /// How many nodes should have this data
    pub replication_factor: u8,

    /// Nodes that have acknowledged caching this
    pub replicated_to: Vec<String>,

    /// Access count (for LRU eviction)
    access_count: u64,

    /// Last accessed time
    last_accessed: DateTime<Utc>,
}

impl CacheEntry {
    /// Create a new cache entry
    pub fn new(
        key: &str,
        data: Vec<u8>,
        category: DataCategory,
        origin_node: &str,
        ttl: Option<Duration>,
    ) -> Self {
        let now = Utc::now();
        let expires_at = ttl.map(|d| now + chrono::Duration::from_std(d).unwrap_or(chrono::Duration::hours(24)));

        Self {
            key: key.to_string(),
            data,
            category,
            origin_node: origin_node.to_string(),
            version: 1,
            created_at: now,
            expires_at,
            replication_factor: 2, // Default: replicate to 2 nodes
            replicated_to: vec![origin_node.to_string()],
            access_count: 0,
            last_accessed: now,
        }
    }

    /// Check if this entry has expired
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Utc::now() > exp)
            .unwrap_or(false)
    }

    /// Mark this entry as accessed
    pub fn touch(&mut self) {
        self.access_count += 1;
        self.last_accessed = Utc::now();
    }

    /// Update the data and increment version
    pub fn update(&mut self, data: Vec<u8>) {
        self.data = data;
        self.version += 1;
        self.last_accessed = Utc::now();
    }

    /// Get the size of this entry in bytes
    pub fn size(&self) -> usize {
        self.data.len() + self.key.len() + self.origin_node.len()
    }

    /// Check if replication target is met
    pub fn is_fully_replicated(&self) -> bool {
        self.replicated_to.len() >= self.replication_factor as usize
    }

    /// Add a node to the replicated list
    pub fn add_replica(&mut self, node_id: &str) {
        if !self.replicated_to.contains(&node_id.to_string()) {
            self.replicated_to.push(node_id.to_string());
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheStats {
    /// Total entries
    pub entry_count: usize,

    /// Total bytes used
    pub bytes_used: u64,

    /// Cache hits
    pub hits: u64,

    /// Cache misses
    pub misses: u64,

    /// Evictions due to space
    pub evictions: u64,

    /// Expired entries removed
    pub expirations: u64,
}

impl CacheStats {
    /// Calculate hit rate
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total > 0 {
            self.hits as f64 / total as f64
        } else {
            0.0
        }
    }
}

/// Shared cache for the federation mesh
#[derive(Debug)]
pub struct SharedCache {
    /// Local node ID
    node_id: String,

    /// Maximum cache size in bytes
    max_bytes: u64,

    /// Current bytes used
    bytes_used: u64,

    /// The cache entries (category -> key -> entry)
    entries: HashMap<DataCategory, HashMap<String, CacheEntry>>,

    /// Statistics
    stats: CacheStats,

    /// Categories we're willing to cache
    allowed_categories: Vec<DataCategory>,
}

impl SharedCache {
    /// Create a new shared cache
    pub fn new(node_id: &str, max_bytes: u64) -> Self {
        Self {
            node_id: node_id.to_string(),
            max_bytes,
            bytes_used: 0,
            entries: HashMap::new(),
            stats: CacheStats::default(),
            allowed_categories: vec![
                DataCategory::Presence,
                DataCategory::Profile,
                DataCategory::Broadcasts,
            ],
        }
    }

    /// Set which categories this cache accepts
    pub fn set_allowed_categories(&mut self, categories: Vec<DataCategory>) {
        self.allowed_categories = categories;
    }

    /// Check if a category is cacheable here
    pub fn allows_category(&self, category: DataCategory) -> bool {
        self.allowed_categories.contains(&category)
    }

    /// Get an entry from the cache
    pub fn get(&mut self, category: DataCategory, key: &str) -> Option<&CacheEntry> {
        // Check if category exists
        if let Some(cat_map) = self.entries.get_mut(&category) {
            if let Some(entry) = cat_map.get_mut(key) {
                // Check expiration
                if entry.is_expired() {
                    // Remove expired entry
                    self.stats.expirations += 1;
                    self.bytes_used = self.bytes_used.saturating_sub(entry.size() as u64);
                    cat_map.remove(key);
                    self.stats.misses += 1;
                    return None;
                }

                entry.touch();
                self.stats.hits += 1;

                // Return immutable reference (need to re-borrow)
                return self.entries.get(&category)?.get(key);
            }
        }

        self.stats.misses += 1;
        None
    }

    /// Put an entry into the cache
    pub fn put(&mut self, entry: CacheEntry) -> Result<()> {
        // Check if we allow this category
        if !self.allows_category(entry.category) {
            return Err(FederationError::Internal(format!(
                "Category {:?} not allowed in this cache",
                entry.category
            )));
        }

        let entry_size = entry.size() as u64;

        // Check if we need to evict
        while self.bytes_used + entry_size > self.max_bytes {
            if !self.evict_one() {
                return Err(FederationError::Internal(
                    "Cache full, cannot evict".to_string()
                ));
            }
        }

        // Insert the entry
        let category = entry.category;
        let key = entry.key.clone();

        // Remove old entry if exists
        if let Some(cat_map) = self.entries.get_mut(&category) {
            if let Some(old) = cat_map.remove(&key) {
                self.bytes_used = self.bytes_used.saturating_sub(old.size() as u64);
            }
        }

        // Insert new entry
        self.bytes_used += entry_size;
        self.entries
            .entry(category)
            .or_insert_with(HashMap::new)
            .insert(key, entry);

        self.stats.entry_count = self.count_entries();

        Ok(())
    }

    /// Remove an entry from the cache
    pub fn remove(&mut self, category: DataCategory, key: &str) -> Option<CacheEntry> {
        if let Some(cat_map) = self.entries.get_mut(&category) {
            if let Some(entry) = cat_map.remove(key) {
                self.bytes_used = self.bytes_used.saturating_sub(entry.size() as u64);
                self.stats.entry_count = self.count_entries();
                return Some(entry);
            }
        }
        None
    }

    /// Evict one entry (LRU)
    fn evict_one(&mut self) -> bool {
        // Find the least recently accessed entry
        let mut oldest: Option<(DataCategory, String, DateTime<Utc>)> = None;

        for (category, cat_map) in &self.entries {
            for (key, entry) in cat_map {
                match &oldest {
                    None => oldest = Some((*category, key.clone(), entry.last_accessed)),
                    Some((_, _, time)) if entry.last_accessed < *time => {
                        oldest = Some((*category, key.clone(), entry.last_accessed));
                    }
                    _ => {}
                }
            }
        }

        // Remove the oldest entry
        if let Some((category, key, _)) = oldest {
            if let Some(entry) = self.remove(category, &key) {
                self.stats.evictions += 1;
                return true;
            }
        }

        false
    }

    /// Remove all expired entries
    pub fn cleanup_expired(&mut self) -> usize {
        let mut removed = 0;
        let mut to_remove: Vec<(DataCategory, String)> = Vec::new();

        for (category, cat_map) in &self.entries {
            for (key, entry) in cat_map {
                if entry.is_expired() {
                    to_remove.push((*category, key.clone()));
                }
            }
        }

        for (category, key) in to_remove {
            if self.remove(category, &key).is_some() {
                self.stats.expirations += 1;
                removed += 1;
            }
        }

        removed
    }

    /// Get entries that need replication
    pub fn get_under_replicated(&self) -> Vec<&CacheEntry> {
        let mut result = Vec::new();

        for cat_map in self.entries.values() {
            for entry in cat_map.values() {
                if !entry.is_fully_replicated() && !entry.is_expired() {
                    result.push(entry);
                }
            }
        }

        result
    }

    /// Get entries for a sync vector (key, version pairs)
    pub fn get_sync_vector(&self, category: DataCategory) -> Vec<(String, u64)> {
        self.entries
            .get(&category)
            .map(|cat_map| {
                cat_map
                    .iter()
                    .filter(|(_, e)| !e.is_expired())
                    .map(|(k, e)| (k.clone(), e.version))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Count total entries
    fn count_entries(&self) -> usize {
        self.entries.values().map(|m| m.len()).sum()
    }

    /// Get cache statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get bytes used
    pub fn bytes_used(&self) -> u64 {
        self.bytes_used
    }

    /// Get max bytes
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Get utilization percentage
    pub fn utilization(&self) -> f64 {
        if self.max_bytes > 0 {
            self.bytes_used as f64 / self.max_bytes as f64
        } else {
            0.0
        }
    }
}

/// Thread-safe shared cache wrapper
pub type ThreadSafeCache = Arc<RwLock<SharedCache>>;

/// Create a thread-safe cache
pub fn create_cache(node_id: &str, max_bytes: u64) -> ThreadSafeCache {
    Arc::new(RwLock::new(SharedCache::new(node_id, max_bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_put_get() {
        let mut cache = SharedCache::new("test-node", 1024 * 1024);

        let entry = CacheEntry::new(
            "test-key",
            b"hello world".to_vec(),
            DataCategory::Presence,
            "origin-node",
            Some(Duration::from_secs(3600)),
        );

        cache.put(entry).unwrap();

        let retrieved = cache.get(DataCategory::Presence, "test-key");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().data, b"hello world");
    }

    #[test]
    fn test_cache_expiration() {
        let mut cache = SharedCache::new("test-node", 1024 * 1024);

        // Create an already-expired entry
        let mut entry = CacheEntry::new(
            "expired-key",
            b"old data".to_vec(),
            DataCategory::Presence,
            "origin-node",
            None,
        );
        entry.expires_at = Some(Utc::now() - chrono::Duration::hours(1));

        cache.put(entry).unwrap();

        // Should not be retrievable
        let retrieved = cache.get(DataCategory::Presence, "expired-key");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_cache_eviction() {
        // Small cache that can only hold a few entries
        let mut cache = SharedCache::new("test-node", 100);

        // Add entries until we trigger eviction
        for i in 0..10 {
            let entry = CacheEntry::new(
                &format!("key-{}", i),
                vec![0u8; 20], // 20 bytes each
                DataCategory::Presence,
                "origin",
                None,
            );
            let _ = cache.put(entry);
        }

        // Should have evicted some entries
        assert!(cache.stats.evictions > 0);
    }

    #[test]
    fn test_sync_vector() {
        let mut cache = SharedCache::new("test-node", 1024 * 1024);

        for i in 0..5 {
            let mut entry = CacheEntry::new(
                &format!("key-{}", i),
                vec![0u8; 10],
                DataCategory::Presence,
                "origin",
                None,
            );
            entry.version = (i + 1) as u64;
            cache.put(entry).unwrap();
        }

        let sync_vec = cache.get_sync_vector(DataCategory::Presence);
        assert_eq!(sync_vec.len(), 5);
    }

    #[test]
    fn test_category_restriction() {
        let mut cache = SharedCache::new("test-node", 1024 * 1024);
        cache.set_allowed_categories(vec![DataCategory::Presence]);

        // This should fail - DirectMessages not allowed
        let entry = CacheEntry::new(
            "dm-key",
            b"secret".to_vec(),
            DataCategory::DirectMessages,
            "origin",
            None,
        );

        let result = cache.put(entry);
        assert!(result.is_err());
    }
}
