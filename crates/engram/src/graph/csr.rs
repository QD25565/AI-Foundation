//! CSR (Compressed Sparse Row) Graph Storage for Engram Knowledge Graph 2.0
//!
//! CSR is a cache-friendly format for graph storage that provides:
//! - O(V+E) space complexity
//! - O(1) access to all outgoing edges from any node
//! - Cache-friendly sequential memory access during traversal
//! - Efficient serialization/deserialization
//!
//! This implementation extends traditional CSR with:
//! - Typed edges (structural, semantic, causal, temporal)
//! - Edge metadata (weight, confidence, timestamp)
//! - Bidirectional lookup (outgoing and incoming edges)
//! - Dynamic updates with periodic compaction

use std::collections::HashMap;
use super::types::{Edge, EdgeType};

// ============================================================================
// CSR Graph Structure
// ============================================================================

/// CSR (Compressed Sparse Row) graph storage
///
/// Memory layout for outgoing edges:
/// ```text
/// row_offsets: [0, 3, 5, 8, ...]  // Where each node's edges start
/// col_indices: [2, 4, 7, 1, 3, 0, 2, 5, ...]  // Target nodes
/// edge_data:   [e0, e1, e2, e3, e4, ...]  // Edge metadata
/// ```
///
/// For node i, its outgoing edges are at indices [row_offsets[i], row_offsets[i+1])
#[derive(Debug)]
pub struct CsrGraph {
    /// Number of nodes in the graph
    node_count: usize,

    /// Number of edges in the graph
    edge_count: usize,

    // === Outgoing edges (CSR format) ===
    /// Index into col_indices/edge_data where each node's outgoing edges start
    /// Length: node_count + 1 (last element is total edge count)
    out_row_offsets: Vec<usize>,

    /// Target node IDs for each edge (sorted by source node)
    out_col_indices: Vec<u64>,

    /// Edge metadata (parallel array to col_indices)
    out_edge_data: Vec<EdgeData>,

    // === Incoming edges (CSC format) ===
    /// Index into in_col_indices where each node's incoming edges start
    in_row_offsets: Vec<usize>,

    /// Source node IDs for each incoming edge
    in_col_indices: Vec<u64>,

    /// Maps incoming edge index to outgoing edge index for metadata lookup
    in_to_out_map: Vec<usize>,

    // === Node mapping ===
    /// Maps external node IDs (u64) to internal indices (usize)
    node_to_idx: HashMap<u64, usize>,

    /// Maps internal indices back to external node IDs
    idx_to_node: Vec<u64>,

    // === Dynamic update buffer ===
    /// Pending edges that haven't been compacted into CSR yet
    pending_edges: Vec<Edge>,

    /// Threshold for triggering compaction
    compaction_threshold: usize,
}

/// Magic bytes identifying v2 CsrGraph serialization format (with temporal validity)
const CSR_V2_MAGIC: &[u8; 4] = b"EGV2";

/// Compact edge data stored in CSR arrays
#[derive(Debug, Clone)]
pub struct EdgeData {
    /// Edge type (stored as byte for compactness)
    pub edge_type: u8,
    /// Relationship strength (0.0 - 1.0)
    pub weight: f32,
    /// Confidence in this edge (0.0 - 1.0)
    pub confidence: f32,
    /// Unix timestamp of creation (seconds)
    pub timestamp: i64,
    /// Was this edge inferred?
    pub inferred: bool,
    /// When this edge became valid (Unix timestamp seconds, 0 = from creation)
    pub t_valid: i64,
    /// When this edge expired (Unix timestamp seconds, 0 = never expires)
    pub t_invalid: i64,
}

impl EdgeData {
    /// Create from a full Edge
    pub fn from_edge(edge: &Edge) -> Self {
        Self {
            edge_type: edge.edge_type.to_byte(),
            weight: edge.weight,
            confidence: edge.confidence,
            timestamp: edge.timestamp,
            inferred: edge.inferred,
            t_valid: edge.t_valid,
            t_invalid: edge.t_invalid.unwrap_or(0),
        }
    }

    /// Get the EdgeType
    pub fn edge_type(&self) -> Option<EdgeType> {
        EdgeType::from_byte(self.edge_type)
    }

    /// Calculate effective strength
    pub fn effective_strength(&self) -> f32 {
        let type_factor = self.edge_type()
            .map(|t| t.confidence_factor())
            .unwrap_or(0.5);
        self.weight * self.confidence * type_factor
    }

    /// Check if this edge is valid at the given Unix timestamp (seconds)
    /// t_valid == 0 means "valid from beginning of time" (backward compat default)
    /// t_invalid == 0 means "never expires" (backward compat default)
    pub fn is_valid_at(&self, now: i64) -> bool {
        (self.t_valid == 0 || self.t_valid <= now)
            && (self.t_invalid == 0 || self.t_invalid > now)
    }

    /// Serialize to bytes (v2 format: 34 bytes fixed)
    /// Layout: edge_type(1) + weight(4) + confidence(4) + timestamp(8) + inferred(1) + t_valid(8) + t_invalid(8)
    pub fn to_bytes(&self) -> [u8; 34] {
        let mut bytes = [0u8; 34];
        bytes[0] = self.edge_type;
        bytes[1..5].copy_from_slice(&self.weight.to_le_bytes());
        bytes[5..9].copy_from_slice(&self.confidence.to_le_bytes());
        bytes[9..17].copy_from_slice(&self.timestamp.to_le_bytes());
        bytes[17] = if self.inferred { 1 } else { 0 };
        bytes[18..26].copy_from_slice(&self.t_valid.to_le_bytes());
        bytes[26..34].copy_from_slice(&self.t_invalid.to_le_bytes());
        bytes
    }

    /// Deserialize from v1 bytes (17 bytes, no temporal fields)
    /// Used for reading old .engram files
    pub fn from_bytes_v1(bytes: &[u8; 17]) -> Self {
        // v1 layout has a bug: byte 16 is the last byte of timestamp AND inferred
        // In practice timestamp MSB is always 0 for current Unix times, so no data loss
        let timestamp = i64::from_le_bytes(bytes[9..17].try_into().unwrap());
        Self {
            edge_type: bytes[0],
            weight: f32::from_le_bytes(bytes[1..5].try_into().unwrap()),
            confidence: f32::from_le_bytes(bytes[5..9].try_into().unwrap()),
            timestamp,
            inferred: bytes[16] != 0,
            // Old edges default: valid from creation, never expire
            t_valid: 0,
            t_invalid: 0,
        }
    }

    /// Deserialize from v2 bytes (34 bytes, with temporal fields)
    pub fn from_bytes_v2(bytes: &[u8; 34]) -> Self {
        Self {
            edge_type: bytes[0],
            weight: f32::from_le_bytes(bytes[1..5].try_into().unwrap()),
            confidence: f32::from_le_bytes(bytes[5..9].try_into().unwrap()),
            timestamp: i64::from_le_bytes(bytes[9..17].try_into().unwrap()),
            inferred: bytes[17] != 0,
            t_valid: i64::from_le_bytes(bytes[18..26].try_into().unwrap()),
            t_invalid: i64::from_le_bytes(bytes[26..34].try_into().unwrap()),
        }
    }
}

// ============================================================================
// CSR Graph Implementation
// ============================================================================

impl CsrGraph {
    /// Create an empty CSR graph
    pub fn new() -> Self {
        Self::with_capacity(0, 0)
    }

    /// Create with expected capacity
    pub fn with_capacity(expected_nodes: usize, expected_edges: usize) -> Self {
        Self {
            node_count: 0,
            edge_count: 0,
            out_row_offsets: vec![0],
            out_col_indices: Vec::with_capacity(expected_edges),
            out_edge_data: Vec::with_capacity(expected_edges),
            in_row_offsets: vec![0],
            in_col_indices: Vec::with_capacity(expected_edges),
            in_to_out_map: Vec::with_capacity(expected_edges),
            node_to_idx: HashMap::with_capacity(expected_nodes),
            idx_to_node: Vec::with_capacity(expected_nodes),
            pending_edges: Vec::new(),
            compaction_threshold: 1000,
        }
    }

    /// Build CSR graph from a list of edges (batch construction)
    pub fn from_edges(edges: &[Edge]) -> Self {
        if edges.is_empty() {
            return Self::new();
        }

        // Collect all unique nodes
        let mut nodes: Vec<u64> = edges.iter()
            .flat_map(|e| [e.source, e.target])
            .collect();
        nodes.sort_unstable();
        nodes.dedup();

        let node_count = nodes.len();
        let edge_count = edges.len();

        // Build node mapping
        let mut node_to_idx: HashMap<u64, usize> = HashMap::with_capacity(node_count);
        for (idx, &node) in nodes.iter().enumerate() {
            node_to_idx.insert(node, idx);
        }

        // Sort edges by source node for CSR construction
        let mut sorted_edges: Vec<&Edge> = edges.iter().collect();
        sorted_edges.sort_by_key(|e| node_to_idx.get(&e.source).unwrap_or(&0));

        // Build outgoing CSR arrays
        let mut out_row_offsets = vec![0usize; node_count + 1];
        let mut out_col_indices = Vec::with_capacity(edge_count);
        let mut out_edge_data = Vec::with_capacity(edge_count);

        // Count edges per source node
        for edge in &sorted_edges {
            if let Some(&idx) = node_to_idx.get(&edge.source) {
                out_row_offsets[idx + 1] += 1;
            }
        }

        // Convert counts to offsets (prefix sum)
        for i in 1..=node_count {
            out_row_offsets[i] += out_row_offsets[i - 1];
        }

        // Fill edge data
        for edge in &sorted_edges {
            out_col_indices.push(edge.target);
            out_edge_data.push(EdgeData::from_edge(edge));
        }

        // Build incoming CSR arrays (CSC format)
        // Sort by target for incoming edges
        let mut sorted_by_target: Vec<(usize, &Edge)> = sorted_edges.iter()
            .enumerate()
            .map(|(i, e)| (i, *e))
            .collect();
        sorted_by_target.sort_by_key(|(_, e)| node_to_idx.get(&e.target).unwrap_or(&0));

        let mut in_row_offsets = vec![0usize; node_count + 1];
        let mut in_col_indices = Vec::with_capacity(edge_count);
        let mut in_to_out_map = Vec::with_capacity(edge_count);

        // Count incoming edges per target node
        for (_, edge) in &sorted_by_target {
            if let Some(&idx) = node_to_idx.get(&edge.target) {
                in_row_offsets[idx + 1] += 1;
            }
        }

        // Convert to offsets
        for i in 1..=node_count {
            in_row_offsets[i] += in_row_offsets[i - 1];
        }

        // Fill incoming edge data
        for (out_idx, edge) in &sorted_by_target {
            in_col_indices.push(edge.source);
            in_to_out_map.push(*out_idx);
        }

        Self {
            node_count,
            edge_count,
            out_row_offsets,
            out_col_indices,
            out_edge_data,
            in_row_offsets,
            in_col_indices,
            in_to_out_map,
            node_to_idx,
            idx_to_node: nodes,
            pending_edges: Vec::new(),
            compaction_threshold: 1000,
        }
    }

    /// Get the number of nodes
    pub fn node_count(&self) -> usize {
        self.node_count + self.pending_node_count()
    }

    /// Get the number of edges
    pub fn edge_count(&self) -> usize {
        self.edge_count + self.pending_edges.len()
    }

    /// Count pending nodes not yet in main CSR
    fn pending_node_count(&self) -> usize {
        self.pending_edges.iter()
            .flat_map(|e| [e.source, e.target])
            .filter(|id| !self.node_to_idx.contains_key(id))
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Check if a node exists
    pub fn has_node(&self, node_id: u64) -> bool {
        self.node_to_idx.contains_key(&node_id) ||
        self.pending_edges.iter().any(|e| e.source == node_id || e.target == node_id)
    }

    /// Add a new edge (buffered for batch processing)
    pub fn add_edge(&mut self, edge: Edge) {
        self.pending_edges.push(edge);

        // Auto-compact if threshold reached
        if self.pending_edges.len() >= self.compaction_threshold {
            self.compact();
        }
    }

    /// Add multiple edges at once
    pub fn add_edges(&mut self, edges: impl IntoIterator<Item = Edge>) {
        self.pending_edges.extend(edges);

        if self.pending_edges.len() >= self.compaction_threshold {
            self.compact();
        }
    }

    /// Get all outgoing edges from a node
    pub fn outgoing_edges(&self, node_id: u64) -> Vec<(u64, EdgeData)> {
        let mut result = Vec::new();

        // Check main CSR
        if let Some(&idx) = self.node_to_idx.get(&node_id) {
            let start = self.out_row_offsets[idx];
            let end = self.out_row_offsets[idx + 1];

            for i in start..end {
                result.push((self.out_col_indices[i], self.out_edge_data[i].clone()));
            }
        }

        // Check pending edges
        for edge in &self.pending_edges {
            if edge.source == node_id {
                result.push((edge.target, EdgeData::from_edge(edge)));
            }
        }

        result
    }

    /// Get all incoming edges to a node
    pub fn incoming_edges(&self, node_id: u64) -> Vec<(u64, EdgeData)> {
        let mut result = Vec::new();

        // Check main CSR (CSC format)
        if let Some(&idx) = self.node_to_idx.get(&node_id) {
            let start = self.in_row_offsets[idx];
            let end = self.in_row_offsets[idx + 1];

            for i in start..end {
                let out_idx = self.in_to_out_map[i];
                result.push((self.in_col_indices[i], self.out_edge_data[out_idx].clone()));
            }
        }

        // Check pending edges
        for edge in &self.pending_edges {
            if edge.target == node_id {
                result.push((edge.source, EdgeData::from_edge(edge)));
            }
        }

        result
    }

    /// Get all edges of a specific type from a node
    pub fn outgoing_edges_of_type(&self, node_id: u64, edge_type: EdgeType) -> Vec<(u64, EdgeData)> {
        let type_byte = edge_type.to_byte();
        self.outgoing_edges(node_id)
            .into_iter()
            .filter(|(_, data)| data.edge_type == type_byte)
            .collect()
    }

    /// Get all incoming edges of a specific type to a node
    pub fn incoming_edges_of_type(&self, node_id: u64, edge_type: EdgeType) -> Vec<(u64, EdgeData)> {
        let type_byte = edge_type.to_byte();
        self.incoming_edges(node_id)
            .into_iter()
            .filter(|(_, data)| data.edge_type == type_byte)
            .collect()
    }

    /// Get outgoing edges from a node that are currently valid (t_valid <= now, not yet expired)
    /// Use this instead of outgoing_edges() in recall scoring to filter stale edges
    pub fn valid_outgoing_edges(&self, node_id: u64, now: i64) -> Vec<(u64, EdgeData)> {
        self.outgoing_edges(node_id)
            .into_iter()
            .filter(|(_, data)| data.is_valid_at(now))
            .collect()
    }

    /// Invalidate all edges between source and target by setting t_invalid = now
    /// Returns the number of edges invalidated
    /// After calling this, persist_indexes() must be called to save the change
    pub fn invalidate_edge(&mut self, source: u64, target: u64, now: i64) -> usize {
        let mut count = 0;

        // Invalidate in pending_edges (mutable in place)
        for edge in self.pending_edges.iter_mut() {
            if edge.source == source && edge.target == target && edge.t_invalid.is_none() {
                edge.t_invalid = Some(now);
                count += 1;
            }
        }

        // Invalidate in main CSR arrays (mutable in place)
        if let Some(&src_idx) = self.node_to_idx.get(&source) {
            let start = self.out_row_offsets[src_idx];
            let end = self.out_row_offsets[src_idx + 1];
            for i in start..end {
                if self.out_col_indices[i] == target && self.out_edge_data[i].t_invalid == 0 {
                    self.out_edge_data[i].t_invalid = now;
                    count += 1;
                }
            }
        }

        count
    }

    /// Get all neighbors (both directions)
    pub fn neighbors(&self, node_id: u64) -> Vec<u64> {
        let mut neighbors: Vec<u64> = self.outgoing_edges(node_id)
            .into_iter()
            .map(|(target, _)| target)
            .collect();

        neighbors.extend(
            self.incoming_edges(node_id)
                .into_iter()
                .map(|(source, _)| source)
        );

        neighbors.sort_unstable();
        neighbors.dedup();
        neighbors
    }

    /// Get out-degree of a node
    pub fn out_degree(&self, node_id: u64) -> usize {
        let csr_degree = if let Some(&idx) = self.node_to_idx.get(&node_id) {
            self.out_row_offsets[idx + 1] - self.out_row_offsets[idx]
        } else {
            0
        };

        let pending_degree = self.pending_edges.iter()
            .filter(|e| e.source == node_id)
            .count();

        csr_degree + pending_degree
    }

    /// Get in-degree of a node
    pub fn in_degree(&self, node_id: u64) -> usize {
        let csr_degree = if let Some(&idx) = self.node_to_idx.get(&node_id) {
            self.in_row_offsets[idx + 1] - self.in_row_offsets[idx]
        } else {
            0
        };

        let pending_degree = self.pending_edges.iter()
            .filter(|e| e.target == node_id)
            .count();

        csr_degree + pending_degree
    }

    /// Check if an edge exists between two nodes
    pub fn has_edge(&self, source: u64, target: u64) -> bool {
        // Check main CSR
        if let Some(&idx) = self.node_to_idx.get(&source) {
            let start = self.out_row_offsets[idx];
            let end = self.out_row_offsets[idx + 1];

            for i in start..end {
                if self.out_col_indices[i] == target {
                    return true;
                }
            }
        }

        // Check pending
        self.pending_edges.iter()
            .any(|e| e.source == source && e.target == target)
    }

    /// Check if an edge of specific type exists
    pub fn has_edge_of_type(&self, source: u64, target: u64, edge_type: EdgeType) -> bool {
        let type_byte = edge_type.to_byte();

        // Check main CSR
        if let Some(&idx) = self.node_to_idx.get(&source) {
            let start = self.out_row_offsets[idx];
            let end = self.out_row_offsets[idx + 1];

            for i in start..end {
                if self.out_col_indices[i] == target &&
                   self.out_edge_data[i].edge_type == type_byte {
                    return true;
                }
            }
        }

        // Check pending
        self.pending_edges.iter()
            .any(|e| e.source == source && e.target == target &&
                 e.edge_type.to_byte() == type_byte)
    }

    /// Get all edges between two nodes
    pub fn get_edges(&self, source: u64, target: u64) -> Vec<EdgeData> {
        let mut result = Vec::new();

        // Check main CSR
        if let Some(&idx) = self.node_to_idx.get(&source) {
            let start = self.out_row_offsets[idx];
            let end = self.out_row_offsets[idx + 1];

            for i in start..end {
                if self.out_col_indices[i] == target {
                    result.push(self.out_edge_data[i].clone());
                }
            }
        }

        // Check pending
        for edge in &self.pending_edges {
            if edge.source == source && edge.target == target {
                result.push(EdgeData::from_edge(edge));
            }
        }

        result
    }

    /// Compact pending edges into main CSR structure
    pub fn compact(&mut self) {
        if self.pending_edges.is_empty() {
            return;
        }

        // Collect all current edges
        let mut all_edges: Vec<Edge> = Vec::with_capacity(self.edge_count + self.pending_edges.len());

        // Convert existing CSR edges to Edge structs
        for (idx, &node_id) in self.idx_to_node.iter().enumerate() {
            let start = self.out_row_offsets[idx];
            let end = self.out_row_offsets[idx + 1];

            for i in start..end {
                let target = self.out_col_indices[i];
                let data = &self.out_edge_data[i];

                if let Some(edge_type) = data.edge_type() {
                    all_edges.push(Edge {
                        source: node_id,
                        target,
                        edge_type,
                        weight: data.weight,
                        confidence: data.confidence,
                        timestamp: data.timestamp,
                        inferred: data.inferred,
                        inference_chain: None, // Lost during compaction - could preserve if needed
                        t_valid: data.t_valid,
                        t_invalid: if data.t_invalid == 0 { None } else { Some(data.t_invalid) },
                    });
                }
            }
        }

        // Add pending edges
        all_edges.append(&mut self.pending_edges);

        // Rebuild from all edges
        *self = Self::from_edges(&all_edges);
    }

    /// Force rebuild of CSR structure
    pub fn rebuild(&mut self) {
        self.compact();
    }

    /// Get all node IDs in the graph
    pub fn nodes(&self) -> Vec<u64> {
        let mut nodes = self.idx_to_node.clone();

        // Add any new nodes from pending edges
        for edge in &self.pending_edges {
            if !self.node_to_idx.contains_key(&edge.source) && !nodes.contains(&edge.source) {
                nodes.push(edge.source);
            }
            if !self.node_to_idx.contains_key(&edge.target) && !nodes.contains(&edge.target) {
                nodes.push(edge.target);
            }
        }

        nodes
    }

    /// Iterate over all edges
    pub fn iter_edges(&self) -> impl Iterator<Item = (u64, u64, EdgeData)> + '_ {
        // CSR edges
        let csr_iter = self.idx_to_node.iter().enumerate().flat_map(move |(idx, &source)| {
            let start = self.out_row_offsets[idx];
            let end = self.out_row_offsets[idx + 1];
            (start..end).map(move |i| {
                (source, self.out_col_indices[i], self.out_edge_data[i].clone())
            })
        });

        // Pending edges
        let pending_iter = self.pending_edges.iter().map(|e| {
            (e.source, e.target, EdgeData::from_edge(e))
        });

        csr_iter.chain(pending_iter)
    }

    /// Get memory usage statistics
    pub fn memory_stats(&self) -> CsrMemoryStats {
        CsrMemoryStats {
            node_count: self.node_count,
            edge_count: self.edge_count,
            pending_edges: self.pending_edges.len(),
            row_offsets_bytes: self.out_row_offsets.len() * std::mem::size_of::<usize>() * 2,
            col_indices_bytes: self.out_col_indices.len() * std::mem::size_of::<u64>() * 2,
            edge_data_bytes: self.out_edge_data.len() * std::mem::size_of::<EdgeData>(),
            node_map_bytes: self.node_to_idx.len() * (std::mem::size_of::<u64>() + std::mem::size_of::<usize>()),
        }
    }

    // ========================================================================
    // Serialization
    // ========================================================================

    /// Serialize the CSR graph to bytes (v2 format with EGV2 magic header)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // V2 format marker: 4-byte magic + 4 reserved bytes
        bytes.extend_from_slice(CSR_V2_MAGIC);
        bytes.extend_from_slice(&[0u8; 4]); // reserved

        // Header: node_count (8) + edge_count (8) + pending_count (8)
        bytes.extend_from_slice(&(self.node_count as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.edge_count as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.pending_edges.len() as u64).to_le_bytes());

        // Node mapping: idx_to_node
        for &node_id in &self.idx_to_node {
            bytes.extend_from_slice(&node_id.to_le_bytes());
        }

        // Outgoing row offsets
        for &offset in &self.out_row_offsets {
            bytes.extend_from_slice(&(offset as u64).to_le_bytes());
        }

        // Outgoing col indices
        for &col in &self.out_col_indices {
            bytes.extend_from_slice(&col.to_le_bytes());
        }

        // Outgoing edge data (v2: 34 bytes per edge)
        for data in &self.out_edge_data {
            bytes.extend_from_slice(&data.to_bytes());
        }

        // Incoming row offsets
        for &offset in &self.in_row_offsets {
            bytes.extend_from_slice(&(offset as u64).to_le_bytes());
        }

        // Incoming col indices
        for &col in &self.in_col_indices {
            bytes.extend_from_slice(&col.to_le_bytes());
        }

        // Incoming to outgoing map
        for &idx in &self.in_to_out_map {
            bytes.extend_from_slice(&(idx as u64).to_le_bytes());
        }

        // Pending edges (full Edge serialization, includes t_valid/t_invalid)
        for edge in &self.pending_edges {
            bytes.extend_from_slice(&edge.to_bytes());
        }

        bytes
    }

    /// Deserialize from bytes
    /// Handles both v1 format (17-byte EdgeData, no magic header) and
    /// v2 format (EGV2 magic header, 34-byte EdgeData with temporal validity)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 24 {
            return None;
        }

        // Detect format: v2 starts with "EGV2" magic, v1 starts with node_count
        let is_v2 = bytes.len() >= 8 && &bytes[0..4] == CSR_V2_MAGIC;
        let mut offset = if is_v2 { 8 } else { 0 }; // skip magic+reserved for v2
        let edge_data_size = if is_v2 { 34usize } else { 17usize };

        if bytes.len() < offset + 24 {
            return None;
        }

        // Header
        let node_count = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
        offset += 8;
        let edge_count = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
        offset += 8;
        let pending_count = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
        offset += 8;

        // Node mapping
        let mut idx_to_node = Vec::with_capacity(node_count);
        let mut node_to_idx = HashMap::with_capacity(node_count);
        for i in 0..node_count {
            let node_id = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?);
            offset += 8;
            idx_to_node.push(node_id);
            node_to_idx.insert(node_id, i);
        }

        // Outgoing row offsets (node_count + 1 entries)
        let mut out_row_offsets = Vec::with_capacity(node_count + 1);
        for _ in 0..=node_count {
            let off = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
            offset += 8;
            out_row_offsets.push(off);
        }

        // Outgoing col indices
        let mut out_col_indices = Vec::with_capacity(edge_count);
        for _ in 0..edge_count {
            let col = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?);
            offset += 8;
            out_col_indices.push(col);
        }

        // Outgoing edge data (17 bytes for v1, 34 bytes for v2)
        let mut out_edge_data = Vec::with_capacity(edge_count);
        for _ in 0..edge_count {
            if offset + edge_data_size > bytes.len() {
                return None;
            }
            let data = if is_v2 {
                let data_bytes: [u8; 34] = bytes[offset..offset+34].try_into().ok()?;
                EdgeData::from_bytes_v2(&data_bytes)
            } else {
                let data_bytes: [u8; 17] = bytes[offset..offset+17].try_into().ok()?;
                EdgeData::from_bytes_v1(&data_bytes)
            };
            offset += edge_data_size;
            out_edge_data.push(data);
        }

        // Incoming row offsets
        let mut in_row_offsets = Vec::with_capacity(node_count + 1);
        for _ in 0..=node_count {
            let off = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
            offset += 8;
            in_row_offsets.push(off);
        }

        // Incoming col indices
        let mut in_col_indices = Vec::with_capacity(edge_count);
        for _ in 0..edge_count {
            let col = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?);
            offset += 8;
            in_col_indices.push(col);
        }

        // Incoming to outgoing map
        let mut in_to_out_map = Vec::with_capacity(edge_count);
        for _ in 0..edge_count {
            let idx = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
            offset += 8;
            in_to_out_map.push(idx);
        }

        // Pending edges (full Edge serialization — from_bytes auto-detects v1/v2 by size)
        let mut pending_edges = Vec::with_capacity(pending_count);
        for _ in 0..pending_count {
            if offset >= bytes.len() {
                break;
            }
            if let Some(edge) = Edge::from_bytes(&bytes[offset..]) {
                let size = edge.byte_size();
                offset += size;
                pending_edges.push(edge);
            }
        }

        Some(Self {
            node_count,
            edge_count,
            out_row_offsets,
            out_col_indices,
            out_edge_data,
            in_row_offsets,
            in_col_indices,
            in_to_out_map,
            node_to_idx,
            idx_to_node,
            pending_edges,
            compaction_threshold: 1000,
        })
    }
}

impl Default for CsrGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Memory Statistics
// ============================================================================

/// Memory usage statistics for CSR graph
#[derive(Debug, Clone)]
pub struct CsrMemoryStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub pending_edges: usize,
    pub row_offsets_bytes: usize,
    pub col_indices_bytes: usize,
    pub edge_data_bytes: usize,
    pub node_map_bytes: usize,
}

impl CsrMemoryStats {
    /// Total memory usage in bytes
    pub fn total_bytes(&self) -> usize {
        self.row_offsets_bytes + self.col_indices_bytes +
        self.edge_data_bytes + self.node_map_bytes
    }

    /// Bytes per edge (average)
    pub fn bytes_per_edge(&self) -> f64 {
        if self.edge_count == 0 {
            0.0
        } else {
            self.total_bytes() as f64 / self.edge_count as f64
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{CausalEdge, SemanticEdge, StructuralEdge, TemporalEdge};

    #[test]
    fn test_empty_graph() {
        let graph = CsrGraph::new();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_from_edges() {
        let edges = vec![
            Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::IsA), 0.9),
            Edge::new(1, 3, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.8),
            Edge::new(2, 3, EdgeType::Causal(CausalEdge::Causes), 0.7),
        ];

        let graph = CsrGraph::from_edges(&edges);

        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 3);

        // Check outgoing edges
        let out_1 = graph.outgoing_edges(1);
        assert_eq!(out_1.len(), 2);

        let out_2 = graph.outgoing_edges(2);
        assert_eq!(out_2.len(), 1);

        // Check incoming edges
        let in_3 = graph.incoming_edges(3);
        assert_eq!(in_3.len(), 2);
    }

    #[test]
    fn test_dynamic_add_edge() {
        let mut graph = CsrGraph::new();

        graph.add_edge(Edge::new(1, 2, EdgeType::Structural(StructuralEdge::References), 1.0));
        graph.add_edge(Edge::new(2, 3, EdgeType::Temporal(TemporalEdge::Before), 1.0));

        assert_eq!(graph.edge_count(), 2);
        assert!(graph.has_edge(1, 2));
        assert!(graph.has_edge(2, 3));
        assert!(!graph.has_edge(1, 3));
    }

    #[test]
    fn test_compact() {
        let mut graph = CsrGraph::new();

        // Add edges (will be in pending buffer)
        for i in 0..10 {
            graph.add_edge(Edge::new(i, i + 1, EdgeType::Semantic(SemanticEdge::SimilarTo), 0.5));
        }

        // Compact to CSR
        graph.compact();

        assert_eq!(graph.pending_edges.len(), 0);
        assert_eq!(graph.edge_count, 10);

        // Verify edges still accessible
        assert!(graph.has_edge(0, 1));
        assert!(graph.has_edge(9, 10));
    }

    #[test]
    fn test_degrees() {
        let edges = vec![
            Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::IsA), 1.0),
            Edge::new(1, 3, EdgeType::Semantic(SemanticEdge::IsA), 1.0),
            Edge::new(1, 4, EdgeType::Semantic(SemanticEdge::IsA), 1.0),
            Edge::new(5, 1, EdgeType::Semantic(SemanticEdge::PartOf), 1.0),
        ];

        let graph = CsrGraph::from_edges(&edges);

        assert_eq!(graph.out_degree(1), 3);
        assert_eq!(graph.in_degree(1), 1);
    }

    #[test]
    fn test_edge_type_filter() {
        let edges = vec![
            Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::IsA), 1.0),
            Edge::new(1, 3, EdgeType::Causal(CausalEdge::Causes), 1.0),
            Edge::new(1, 4, EdgeType::Semantic(SemanticEdge::IsA), 1.0),
        ];

        let graph = CsrGraph::from_edges(&edges);

        let isa_edges = graph.outgoing_edges_of_type(1, EdgeType::Semantic(SemanticEdge::IsA));
        assert_eq!(isa_edges.len(), 2);

        let causes_edges = graph.outgoing_edges_of_type(1, EdgeType::Causal(CausalEdge::Causes));
        assert_eq!(causes_edges.len(), 1);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let edges = vec![
            Edge::new(100, 200, EdgeType::Structural(StructuralEdge::Contains), 0.95),
            Edge::new(200, 300, EdgeType::Temporal(TemporalEdge::After), 0.8),
        ];

        let graph = CsrGraph::from_edges(&edges);
        let bytes = graph.to_bytes();
        let recovered = CsrGraph::from_bytes(&bytes).expect("Should deserialize");

        assert_eq!(graph.node_count(), recovered.node_count());
        assert_eq!(graph.edge_count(), recovered.edge_count());
        assert!(recovered.has_edge(100, 200));
        assert!(recovered.has_edge(200, 300));
    }

    #[test]
    fn test_neighbors() {
        let edges = vec![
            Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::RelatedTo), 1.0),
            Edge::new(1, 3, EdgeType::Semantic(SemanticEdge::RelatedTo), 1.0),
            Edge::new(4, 1, EdgeType::Semantic(SemanticEdge::RelatedTo), 1.0),
        ];

        let graph = CsrGraph::from_edges(&edges);

        let neighbors = graph.neighbors(1);
        assert_eq!(neighbors.len(), 3); // 2, 3, 4
        assert!(neighbors.contains(&2));
        assert!(neighbors.contains(&3));
        assert!(neighbors.contains(&4));
    }

    #[test]
    fn test_memory_stats() {
        let edges: Vec<Edge> = (0..100)
            .map(|i| Edge::new(i, i + 1, EdgeType::Semantic(SemanticEdge::SimilarTo), 0.5))
            .collect();

        let graph = CsrGraph::from_edges(&edges);
        let stats = graph.memory_stats();

        assert_eq!(stats.node_count, 101);
        assert_eq!(stats.edge_count, 100);
        assert!(stats.total_bytes() > 0);
        assert!(stats.bytes_per_edge() > 0.0);
    }
}
