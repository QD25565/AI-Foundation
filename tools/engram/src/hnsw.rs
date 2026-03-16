//! HNSW (Hierarchical Navigable Small World) index
//!
//! Approximate nearest neighbor search with O(log n) query time.

use crate::vector::{quantize, quantized_cosine_similarity, QuantizedVector};
use rand::Rng;
use std::collections::{BinaryHeap, HashSet};

/// HNSW configuration
#[derive(Debug, Clone)]
pub struct HnswConfig {
    /// Maximum connections per node at layer 0
    pub m: usize,
    /// Maximum connections per node at higher layers
    pub m_max: usize,
    /// Size of dynamic candidate list during construction
    pub ef_construction: usize,
    /// Multiplier for level generation (1/ln(M))
    pub ml: f64,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: 16,
            m_max: 32,
            ef_construction: 200,
            ml: 1.0 / (16.0_f64).ln(),
        }
    }
}

/// A node in the HNSW graph
#[derive(Debug, Clone)]
struct Node {
    /// Note ID this node represents
    id: u64,
    /// Connections at each layer (length - 1 == node's max layer)
    connections: Vec<Vec<u64>>,
}

/// HNSW index for approximate nearest neighbor search
pub struct HnswIndex {
    /// Configuration
    config: HnswConfig,
    /// All nodes
    nodes: Vec<Node>,
    /// ID to index mapping
    id_to_idx: std::collections::HashMap<u64, usize>,
    /// Entry point (top-level node)
    entry_point: Option<usize>,
    /// Maximum layer in the graph
    max_layer: usize,
    /// Quantized vectors for fast distance computation (4x less memory than f32)
    vectors: std::collections::HashMap<u64, QuantizedVector>,
}

impl HnswIndex {
    /// Create a new HNSW index
    pub fn new() -> Self {
        Self::with_config(HnswConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: HnswConfig) -> Self {
        Self {
            config,
            nodes: Vec::new(),
            id_to_idx: std::collections::HashMap::new(),
            entry_point: None,
            max_layer: 0,
            vectors: std::collections::HashMap::new(),
        }
    }

    /// Add a vector to the index (quantized internally for 4x memory savings)
    pub fn add(&mut self, id: u64, vector: &[f32]) {
        // Quantize and store
        let q = quantize(vector);
        self.vectors.insert(id, q.clone());

        // Generate random level
        let level = self.random_level();

        // Create node
        let node = Node {
            id,
            connections: vec![Vec::new(); level + 1],
        };

        let node_idx = self.nodes.len();
        self.nodes.push(node);
        self.id_to_idx.insert(id, node_idx);

        // If first node, set as entry point
        if self.entry_point.is_none() {
            self.entry_point = Some(node_idx);
            self.max_layer = level;
            return;
        }

        let entry_point = self.entry_point.unwrap();

        // Find entry point for insertion
        let mut current = entry_point;

        // Search from top layer to level+1
        for layer in (level + 1..=self.max_layer).rev() {
            current = self.search_layer_single(&q, current, layer);
        }

        // Insert at each layer from level down to 0
        for layer in (0..=level.min(self.max_layer)).rev() {
            let neighbors = self.search_layer(&q, current, self.config.ef_construction, layer);

            // Select M best neighbors
            let selected: Vec<u64> = neighbors
                .into_iter()
                .take(if layer == 0 { self.config.m_max } else { self.config.m })
                .map(|(id, _)| id)
                .collect();

            // Add bidirectional connections
            self.nodes[node_idx].connections[layer] = selected.clone();

            for &neighbor_id in &selected {
                if let Some(&neighbor_idx) = self.id_to_idx.get(&neighbor_id) {
                    let max_conn = if layer == 0 { self.config.m_max } else { self.config.m };

                    if self.nodes[neighbor_idx].connections.len() > layer {
                        self.nodes[neighbor_idx].connections[layer].push(id);

                        // Prune if too many connections
                        if self.nodes[neighbor_idx].connections[layer].len() > max_conn {
                            self.prune_connections(neighbor_idx, layer, max_conn);
                        }
                    }
                }
            }

            if !selected.is_empty() {
                if let Some(&idx) = self.id_to_idx.get(&selected[0]) {
                    current = idx;
                }
            }
        }

        // Update entry point if new node has higher level
        if level > self.max_layer {
            self.entry_point = Some(node_idx);
            self.max_layer = level;
        }
    }

    /// Search for k nearest neighbors (query quantized once, then pure integer arithmetic)
    pub fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<(u64, f32)> {
        if self.entry_point.is_none() {
            return Vec::new();
        }

        let query_q = quantize(query);
        let entry_point = self.entry_point.unwrap();
        let mut current = entry_point;

        // Greedy search from top to layer 1
        for layer in (1..=self.max_layer).rev() {
            current = self.search_layer_single(&query_q, current, layer);
        }

        // Search layer 0 with ef candidates
        let candidates = self.search_layer(&query_q, current, ef.max(k), 0);

        candidates.into_iter().take(k).collect()
    }

    /// Generate random level for new node
    fn random_level(&self) -> usize {
        let mut rng = rand::thread_rng();
        let mut level = 0;

        while rng.gen::<f64>() < self.config.ml && level < 16 {
            level += 1;
        }

        level
    }

    /// Search a single layer, return best single neighbor
    fn search_layer_single(&self, query: &QuantizedVector, entry: usize, layer: usize) -> usize {
        let mut current = entry;
        let mut current_dist = self.distance(query, self.nodes[current].id);

        loop {
            let mut changed = false;

            if layer < self.nodes[current].connections.len() {
                for &neighbor_id in &self.nodes[current].connections[layer] {
                    let dist = self.distance(query, neighbor_id);
                    if dist < current_dist {
                        if let Some(&idx) = self.id_to_idx.get(&neighbor_id) {
                            current = idx;
                            current_dist = dist;
                            changed = true;
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }

        current
    }

    /// Search a layer, return ef nearest neighbors
    fn search_layer(&self, query: &QuantizedVector, entry: usize, ef: usize, layer: usize) -> Vec<(u64, f32)> {
        let mut visited = HashSet::new();
        let mut candidates = BinaryHeap::new();
        let mut results = BinaryHeap::new();

        let entry_id = self.nodes[entry].id;
        let entry_dist = self.distance(query, entry_id);

        visited.insert(entry_id);
        candidates.push(MinHeapEntry { dist: entry_dist, id: entry_id });
        results.push(MaxHeapEntry { dist: entry_dist, id: entry_id });

        while let Some(MinHeapEntry { dist: c_dist, id: c_id }) = candidates.pop() {
            let worst_dist = results.peek().map(|e| e.dist).unwrap_or(f32::MAX);

            if c_dist > worst_dist {
                break;
            }

            if let Some(&c_idx) = self.id_to_idx.get(&c_id) {
                if layer < self.nodes[c_idx].connections.len() {
                    for &neighbor_id in &self.nodes[c_idx].connections[layer] {
                        if visited.insert(neighbor_id) {
                            let dist = self.distance(query, neighbor_id);
                            let worst = results.peek().map(|e| e.dist).unwrap_or(f32::MAX);

                            if dist < worst || results.len() < ef {
                                candidates.push(MinHeapEntry { dist, id: neighbor_id });
                                results.push(MaxHeapEntry { dist, id: neighbor_id });

                                if results.len() > ef {
                                    results.pop();
                                }
                            }
                        }
                    }
                }
            }
        }

        results
            .into_sorted_vec()
            .into_iter()
            .map(|e| (e.id, 1.0 - e.dist)) // Convert distance to similarity
            .collect()
    }

    /// Compute distance (1 - cosine_similarity) using quantized integer arithmetic
    fn distance(&self, query: &QuantizedVector, id: u64) -> f32 {
        if let Some(vec) = self.vectors.get(&id) {
            1.0 - quantized_cosine_similarity(query, vec)
        } else {
            f32::MAX
        }
    }

    /// Prune connections to keep only the best
    fn prune_connections(&mut self, node_idx: usize, layer: usize, max_conn: usize) {
        let node_id = self.nodes[node_idx].id;
        let node_q = match self.vectors.get(&node_id) {
            Some(v) => v.clone(),
            None => return,
        };

        let mut scored: Vec<(u64, f32)> = self.nodes[node_idx].connections[layer]
            .iter()
            .map(|&id| (id, self.distance(&node_q, id)))
            .collect();

        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        scored.truncate(max_conn);

        self.nodes[node_idx].connections[layer] = scored.into_iter().map(|(id, _)| id).collect();
    }

    /// Number of vectors in the index
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Is the index empty?
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Repopulate vectors from external VectorStore (f32 → quantized on ingest).
    /// Called after deserializing the graph structure (which doesn't include vectors).
    pub fn repopulate_vectors(&mut self, vectors: &std::collections::HashMap<u64, Vec<f32>>) {
        self.vectors = vectors.iter()
            .map(|(&id, v)| (id, quantize(v)))
            .collect();
    }

    /// Validate that all node IDs in the graph exist in the given set of valid IDs.
    /// Returns the set of invalid (dangling) node IDs, if any.
    /// Used after deserialization to detect deleted notes that the graph still references.
    pub fn validate_node_ids(&self, valid_ids: &std::collections::HashSet<u64>) -> Vec<u64> {
        self.nodes.iter()
            .map(|n| n.id)
            .filter(|id| !valid_ids.contains(id))
            .collect()
    }

    /// Get all node IDs in the graph.
    pub fn node_ids(&self) -> Vec<u64> {
        self.nodes.iter().map(|n| n.id).collect()
    }

    /// Serialize the HNSW graph structure to bytes.
    /// Format:
    ///   magic: 4 bytes "HNSW"
    ///   version: 1 byte (1)
    ///   config: m(2) + m_max(2) + ef_construction(2) = 6 bytes
    ///   entry_point: 8 bytes (u64::MAX = none)
    ///   max_layer: 2 bytes (u16)
    ///   node_count: 4 bytes (u32)
    ///   per node:
    ///     id: 8 bytes (u64)
    ///     layer_count: 1 byte (u8)
    ///     per layer:
    ///       connection_count: 2 bytes (u16)
    ///       connections: count * 8 bytes (u64 each)
    ///
    /// Vectors are NOT included — they are persisted separately in VectorStore
    /// and repopulated via repopulate_vectors() on load.
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // Magic
        data.extend_from_slice(b"HNSW");
        // Version
        data.push(1u8);
        // Config
        data.extend_from_slice(&(self.config.m as u16).to_le_bytes());
        data.extend_from_slice(&(self.config.m_max as u16).to_le_bytes());
        data.extend_from_slice(&(self.config.ef_construction as u16).to_le_bytes());
        // Entry point — store as node ID (not index) for stability across serialization
        let ep = match self.entry_point {
            Some(idx) => self.nodes.get(idx).map_or(u64::MAX, |n| n.id),
            None => u64::MAX,
        };
        data.extend_from_slice(&ep.to_le_bytes());
        // Max layer
        data.extend_from_slice(&(self.max_layer as u16).to_le_bytes());
        // Node count
        data.extend_from_slice(&(self.nodes.len() as u32).to_le_bytes());

        // Nodes
        for node in &self.nodes {
            data.extend_from_slice(&node.id.to_le_bytes());
            data.push(node.connections.len() as u8);
            for layer_conns in &node.connections {
                data.extend_from_slice(&(layer_conns.len() as u16).to_le_bytes());
                for &conn_id in layer_conns {
                    data.extend_from_slice(&conn_id.to_le_bytes());
                }
            }
        }

        data
    }

    /// Deserialize HNSW graph structure from bytes.
    /// Returns None if data is invalid or version mismatch.
    /// After deserialization, call repopulate_vectors() to restore distance calculation ability.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 21 {
            return None; // Too short for header
        }

        let mut pos = 0;

        // Magic
        if &data[pos..pos + 4] != b"HNSW" {
            return None;
        }
        pos += 4;

        // Version
        let version = data[pos];
        if version != 1 {
            return None; // Unknown version
        }
        pos += 1;

        // Config
        let m = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        let m_max = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        let ef_construction = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        let config = HnswConfig {
            m,
            m_max,
            ef_construction,
            ml: 1.0 / (m as f64).ln(),
        };

        // Entry point — stored as node ID, resolved to index after nodes are loaded
        let ep_node_id = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?);
        pos += 8;

        // Max layer
        let max_layer = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        // Node count
        let node_count = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;

        let mut nodes = Vec::with_capacity(node_count);
        let mut id_to_idx = std::collections::HashMap::with_capacity(node_count);

        for idx in 0..node_count {
            if pos + 9 > data.len() {
                return None; // Truncated
            }

            let id = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?);
            pos += 8;

            let layer_count = data[pos] as usize;
            pos += 1;

            let mut connections = Vec::with_capacity(layer_count);
            for _ in 0..layer_count {
                if pos + 2 > data.len() {
                    return None;
                }
                let conn_count = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                pos += 2;

                if pos + conn_count * 8 > data.len() {
                    return None;
                }
                let mut layer_conns = Vec::with_capacity(conn_count);
                for _ in 0..conn_count {
                    let conn_id = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?);
                    pos += 8;
                    layer_conns.push(conn_id);
                }
                connections.push(layer_conns);
            }

            id_to_idx.insert(id, idx);
            nodes.push(Node { id, connections });
        }

        // Resolve entry point node ID to index
        let entry_point = if ep_node_id == u64::MAX {
            None
        } else {
            id_to_idx.get(&ep_node_id).copied()
        };

        Some(Self {
            config,
            nodes,
            id_to_idx,
            entry_point,
            max_layer,
            vectors: std::collections::HashMap::new(), // Repopulated separately
        })
    }
}

impl Default for HnswIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Min-heap entry for candidate set
#[derive(Clone, Copy)]
struct MinHeapEntry {
    dist: f32,
    id: u64,
}

impl Ord for MinHeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.dist.partial_cmp(&self.dist).unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PartialOrd for MinHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for MinHeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist
    }
}

impl Eq for MinHeapEntry {}

/// Max-heap entry for results
#[derive(Clone, Copy)]
struct MaxHeapEntry {
    dist: f32,
    id: u64,
}

impl Ord for MaxHeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.dist.partial_cmp(&other.dist).unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PartialOrd for MaxHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for MaxHeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist
    }
}

impl Eq for MaxHeapEntry {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::DIMS;

    #[test]
    fn test_basic_search() {
        let mut index = HnswIndex::new();

        // Add vectors with different directions (not just magnitude)
        for i in 1..=100 {
            let mut vec = vec![0.0f32; DIMS];
            let angle = (i as f32) * 0.1; // Different angles
            vec[0] = angle.cos();
            vec[1] = angle.sin();
            index.add(i as u64, &vec);
        }

        // Search for vector at angle 5.0 (should match ID 50)
        let target_angle = 5.0_f32;
        let mut query = vec![0.0f32; DIMS];
        query[0] = target_angle.cos();
        query[1] = target_angle.sin();

        let results = index.search(&query, 5, 50);

        assert!(!results.is_empty());
        // ID 50 has angle 50 * 0.1 = 5.0, which matches our query
        assert_eq!(results[0].0, 50, "Closest should be ID 50");
    }

    #[test]
    fn test_empty_index() {
        let index = HnswIndex::new();
        let query = vec![0.0f32; DIMS];

        let results = index.search(&query, 5, 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut index = HnswIndex::new();
        let mut vectors = std::collections::HashMap::new();

        // Add vectors with different directions
        for i in 1..=50 {
            let mut vec = vec![0.0f32; DIMS];
            let angle = (i as f32) * 0.1;
            vec[0] = angle.cos();
            vec[1] = angle.sin();
            index.add(i as u64, &vec);
            vectors.insert(i as u64, vec);
        }

        // Search before serialization
        let mut query = vec![0.0f32; DIMS];
        query[0] = 2.5_f32.cos();
        query[1] = 2.5_f32.sin();
        let results_before = index.search(&query, 5, 50);

        // Serialize
        let data = index.serialize();
        assert!(data.len() > 21); // At least header size

        // Deserialize
        let mut restored = HnswIndex::deserialize(&data).expect("deserialize should succeed");
        restored.repopulate_vectors(&vectors);

        // Verify structure
        assert_eq!(restored.len(), 50);
        assert_eq!(restored.entry_point.is_some(), true);

        // Search after deserialization — should return same results
        let results_after = restored.search(&query, 5, 50);
        assert_eq!(results_before.len(), results_after.len());
        for (before, after) in results_before.iter().zip(results_after.iter()) {
            assert_eq!(before.0, after.0, "Same note IDs in same order");
        }
    }

    #[test]
    fn test_serialize_empty() {
        let index = HnswIndex::new();
        let data = index.serialize();
        let restored = HnswIndex::deserialize(&data).expect("empty deserialize should succeed");
        assert_eq!(restored.len(), 0);
        assert!(restored.is_empty());
    }

    #[test]
    fn test_deserialize_invalid() {
        assert!(HnswIndex::deserialize(b"JUNK").is_none());
        assert!(HnswIndex::deserialize(b"").is_none());
        assert!(HnswIndex::deserialize(b"HNSW\x02").is_none()); // Wrong version
    }
}
