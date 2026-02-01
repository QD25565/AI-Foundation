//! Extended edge types for Engram Knowledge Graph 2.0
//!
//! Provides a rich taxonomy of relationship types for knowledge representation:
//! - Structural: How notes relate organizationally
//! - Semantic: Conceptual relationships
//! - Causal: Cause-effect and logical relationships
//! - Temporal: Time-based relationships
//!
//! Each edge carries weight, confidence, and metadata for inference tracking.

use std::fmt;

// ============================================================================
// Edge Type Taxonomy
// ============================================================================

/// Structural relationships - how notes relate organizationally
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StructuralEdge {
    /// Note A explicitly references/cites note B
    References = 0,
    /// Note A continues a thought or discussion from B
    Continues = 1,
    /// Note A replaces or updates B (B is outdated)
    Supersedes = 2,
    /// Note A contains B (hierarchical containment)
    Contains = 3,
    /// Note A was derived/created from B
    DerivedFrom = 4,
}

/// Semantic relationships - conceptual connections
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SemanticEdge {
    /// Concept hierarchy: A is a type of B (dog IsA animal)
    IsA = 10,
    /// Composition: A is part of B (wheel PartOf car)
    PartOf = 11,
    /// General association between concepts
    RelatedTo = 12,
    /// High semantic similarity (via embeddings)
    SimilarTo = 13,
    /// Same meaning, different expression
    SynonymOf = 14,
    /// Opposite meaning
    AntonymOf = 15,
    /// A is an instance/example of B
    InstanceOf = 16,
    /// A has property/attribute B
    HasProperty = 17,
}

/// Causal relationships - cause-effect and logical connections
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CausalEdge {
    /// A caused B to happen
    Causes = 20,
    /// If A then B (logical implication)
    Implies = 21,
    /// A conflicts with or contradicts B
    Contradicts = 22,
    /// A provides evidence or support for B
    Supports = 23,
    /// A prevents B from happening
    Prevents = 24,
    /// A enables or allows B to happen
    Enables = 25,
    /// A requires B (dependency)
    Requires = 26,
}

/// Temporal relationships - time-based connections
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TemporalEdge {
    /// A happened before B
    Before = 30,
    /// A happened after B
    After = 31,
    /// A happened during B
    During = 32,
    /// A was triggered by B
    TriggeredBy = 33,
    /// A and B happened at the same time
    Concurrent = 34,
    /// Created within temporal proximity window
    TemporalProximity = 35,
}

/// Legacy edge types for backward compatibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LegacyEdge {
    /// High cosine similarity between embeddings (maps to SimilarTo)
    Semantic = 100,
    /// Created within temporal window (maps to TemporalProximity)
    Temporal = 101,
    /// Explicitly linked by user/AI (maps to References)
    Manual = 102,
    /// Shared tag (maps to RelatedTo)
    Tag = 103,
}

// ============================================================================
// Unified Edge Type
// ============================================================================

/// Unified edge type encompassing all relationship categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeType {
    Structural(StructuralEdge),
    Semantic(SemanticEdge),
    Causal(CausalEdge),
    Temporal(TemporalEdge),
    Legacy(LegacyEdge),
}

impl EdgeType {
    /// Convert to a single byte for storage
    pub fn to_byte(&self) -> u8 {
        match self {
            EdgeType::Structural(e) => *e as u8,
            EdgeType::Semantic(e) => *e as u8,
            EdgeType::Causal(e) => *e as u8,
            EdgeType::Temporal(e) => *e as u8,
            EdgeType::Legacy(e) => *e as u8,
        }
    }

    /// Parse from a single byte
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            // Structural (0-9)
            0 => EdgeType::Structural(StructuralEdge::References),
            1 => EdgeType::Structural(StructuralEdge::Continues),
            2 => EdgeType::Structural(StructuralEdge::Supersedes),
            3 => EdgeType::Structural(StructuralEdge::Contains),
            4 => EdgeType::Structural(StructuralEdge::DerivedFrom),

            // Semantic (10-19)
            10 => EdgeType::Semantic(SemanticEdge::IsA),
            11 => EdgeType::Semantic(SemanticEdge::PartOf),
            12 => EdgeType::Semantic(SemanticEdge::RelatedTo),
            13 => EdgeType::Semantic(SemanticEdge::SimilarTo),
            14 => EdgeType::Semantic(SemanticEdge::SynonymOf),
            15 => EdgeType::Semantic(SemanticEdge::AntonymOf),
            16 => EdgeType::Semantic(SemanticEdge::InstanceOf),
            17 => EdgeType::Semantic(SemanticEdge::HasProperty),

            // Causal (20-29)
            20 => EdgeType::Causal(CausalEdge::Causes),
            21 => EdgeType::Causal(CausalEdge::Implies),
            22 => EdgeType::Causal(CausalEdge::Contradicts),
            23 => EdgeType::Causal(CausalEdge::Supports),
            24 => EdgeType::Causal(CausalEdge::Prevents),
            25 => EdgeType::Causal(CausalEdge::Enables),
            26 => EdgeType::Causal(CausalEdge::Requires),

            // Temporal (30-39)
            30 => EdgeType::Temporal(TemporalEdge::Before),
            31 => EdgeType::Temporal(TemporalEdge::After),
            32 => EdgeType::Temporal(TemporalEdge::During),
            33 => EdgeType::Temporal(TemporalEdge::TriggeredBy),
            34 => EdgeType::Temporal(TemporalEdge::Concurrent),
            35 => EdgeType::Temporal(TemporalEdge::TemporalProximity),

            // Legacy (100-103)
            100 => EdgeType::Legacy(LegacyEdge::Semantic),
            101 => EdgeType::Legacy(LegacyEdge::Temporal),
            102 => EdgeType::Legacy(LegacyEdge::Manual),
            103 => EdgeType::Legacy(LegacyEdge::Tag),

            _ => return None,
        })
    }

    /// Check if this edge type is transitive (A->B, B->C implies A->C)
    pub fn is_transitive(&self) -> bool {
        matches!(
            self,
            EdgeType::Semantic(SemanticEdge::IsA)
                | EdgeType::Semantic(SemanticEdge::PartOf)
                | EdgeType::Causal(CausalEdge::Causes)
                | EdgeType::Causal(CausalEdge::Implies)
                | EdgeType::Causal(CausalEdge::Requires)
                | EdgeType::Temporal(TemporalEdge::Before)
                | EdgeType::Temporal(TemporalEdge::After)
        )
    }

    /// Check if this edge type is symmetric (A->B implies B->A)
    pub fn is_symmetric(&self) -> bool {
        matches!(
            self,
            EdgeType::Semantic(SemanticEdge::RelatedTo)
                | EdgeType::Semantic(SemanticEdge::SimilarTo)
                | EdgeType::Semantic(SemanticEdge::SynonymOf)
                | EdgeType::Semantic(SemanticEdge::AntonymOf)
                | EdgeType::Causal(CausalEdge::Contradicts)
                | EdgeType::Temporal(TemporalEdge::Concurrent)
                | EdgeType::Temporal(TemporalEdge::TemporalProximity)
                | EdgeType::Legacy(LegacyEdge::Semantic)
                | EdgeType::Legacy(LegacyEdge::Tag)
        )
    }

    /// Get the inverse edge type (for bidirectional relationships)
    pub fn inverse(&self) -> Option<Self> {
        Some(match self {
            // Symmetric edges are their own inverse
            e if e.is_symmetric() => *e,

            // Structural inverses
            EdgeType::Structural(StructuralEdge::Contains) => {
                EdgeType::Semantic(SemanticEdge::PartOf)
            }
            EdgeType::Structural(StructuralEdge::DerivedFrom) => {
                EdgeType::Structural(StructuralEdge::Supersedes)
            }

            // Semantic inverses
            EdgeType::Semantic(SemanticEdge::IsA) => {
                EdgeType::Semantic(SemanticEdge::InstanceOf)
            }
            EdgeType::Semantic(SemanticEdge::InstanceOf) => {
                EdgeType::Semantic(SemanticEdge::IsA)
            }
            EdgeType::Semantic(SemanticEdge::PartOf) => {
                EdgeType::Structural(StructuralEdge::Contains)
            }

            // Causal inverses
            EdgeType::Causal(CausalEdge::Causes) => {
                EdgeType::Temporal(TemporalEdge::TriggeredBy)
            }
            EdgeType::Causal(CausalEdge::Enables) => {
                EdgeType::Causal(CausalEdge::Requires)
            }
            EdgeType::Causal(CausalEdge::Requires) => {
                EdgeType::Causal(CausalEdge::Enables)
            }

            // Temporal inverses
            EdgeType::Temporal(TemporalEdge::Before) => {
                EdgeType::Temporal(TemporalEdge::After)
            }
            EdgeType::Temporal(TemporalEdge::After) => {
                EdgeType::Temporal(TemporalEdge::Before)
            }
            EdgeType::Temporal(TemporalEdge::TriggeredBy) => {
                EdgeType::Causal(CausalEdge::Causes)
            }

            _ => return None,
        })
    }

    /// Get confidence decay factor for this edge type during inference
    /// Lower values = more uncertainty when inferring through this edge
    pub fn confidence_factor(&self) -> f32 {
        match self {
            // High confidence - definitional relationships
            EdgeType::Semantic(SemanticEdge::IsA) => 0.98,
            EdgeType::Semantic(SemanticEdge::InstanceOf) => 0.98,
            EdgeType::Semantic(SemanticEdge::SynonymOf) => 0.95,

            // Good confidence - structural relationships
            EdgeType::Structural(StructuralEdge::Contains) => 0.95,
            EdgeType::Structural(StructuralEdge::References) => 0.90,
            EdgeType::Structural(StructuralEdge::Supersedes) => 0.92,
            EdgeType::Structural(StructuralEdge::DerivedFrom) => 0.90,
            EdgeType::Structural(StructuralEdge::Continues) => 0.88,

            // Medium confidence - semantic similarity
            EdgeType::Semantic(SemanticEdge::SimilarTo) => 0.80,
            EdgeType::Semantic(SemanticEdge::RelatedTo) => 0.75,
            EdgeType::Semantic(SemanticEdge::PartOf) => 0.90,
            EdgeType::Semantic(SemanticEdge::HasProperty) => 0.85,
            EdgeType::Semantic(SemanticEdge::AntonymOf) => 0.90,

            // Lower confidence - causal claims are inherently uncertain
            EdgeType::Causal(CausalEdge::Causes) => 0.75,
            EdgeType::Causal(CausalEdge::Implies) => 0.85,
            EdgeType::Causal(CausalEdge::Supports) => 0.80,
            EdgeType::Causal(CausalEdge::Contradicts) => 0.85,
            EdgeType::Causal(CausalEdge::Prevents) => 0.70,
            EdgeType::Causal(CausalEdge::Enables) => 0.75,
            EdgeType::Causal(CausalEdge::Requires) => 0.88,

            // Temporal - generally reliable
            EdgeType::Temporal(TemporalEdge::Before) => 0.95,
            EdgeType::Temporal(TemporalEdge::After) => 0.95,
            EdgeType::Temporal(TemporalEdge::During) => 0.90,
            EdgeType::Temporal(TemporalEdge::TriggeredBy) => 0.80,
            EdgeType::Temporal(TemporalEdge::Concurrent) => 0.85,
            EdgeType::Temporal(TemporalEdge::TemporalProximity) => 0.60,

            // Legacy - mapped to reasonable defaults
            EdgeType::Legacy(LegacyEdge::Semantic) => 0.80,
            EdgeType::Legacy(LegacyEdge::Temporal) => 0.60,
            EdgeType::Legacy(LegacyEdge::Manual) => 0.95,
            EdgeType::Legacy(LegacyEdge::Tag) => 0.70,
        }
    }

    /// Get category name for display
    pub fn category(&self) -> &'static str {
        match self {
            EdgeType::Structural(_) => "structural",
            EdgeType::Semantic(_) => "semantic",
            EdgeType::Causal(_) => "causal",
            EdgeType::Temporal(_) => "temporal",
            EdgeType::Legacy(_) => "legacy",
        }
    }
}

impl fmt::Display for EdgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeType::Structural(e) => write!(f, "{:?}", e),
            EdgeType::Semantic(e) => write!(f, "{:?}", e),
            EdgeType::Causal(e) => write!(f, "{:?}", e),
            EdgeType::Temporal(e) => write!(f, "{:?}", e),
            EdgeType::Legacy(e) => write!(f, "{:?}", e),
        }
    }
}

// ============================================================================
// Edge Structure
// ============================================================================

/// A weighted, typed edge with metadata for inference tracking
#[derive(Debug, Clone)]
pub struct Edge {
    /// Source node ID
    pub source: u64,
    /// Target node ID
    pub target: u64,
    /// Type of relationship
    pub edge_type: EdgeType,
    /// Strength of the relationship (0.0 - 1.0)
    pub weight: f32,
    /// Confidence in this edge's correctness (0.0 - 1.0)
    pub confidence: f32,
    /// When this edge was created (Unix timestamp)
    pub timestamp: i64,
    /// Was this edge inferred (true) or explicit (false)?
    pub inferred: bool,
    /// If inferred, the chain of edges that led to this inference
    pub inference_chain: Option<Vec<u64>>,
}

impl Edge {
    /// Create a new explicit (non-inferred) edge
    pub fn new(source: u64, target: u64, edge_type: EdgeType, weight: f32) -> Self {
        Self {
            source,
            target,
            edge_type,
            weight,
            confidence: 1.0,
            timestamp: chrono::Utc::now().timestamp(),
            inferred: false,
            inference_chain: None,
        }
    }

    /// Create a new explicit edge with custom confidence
    pub fn with_confidence(
        source: u64,
        target: u64,
        edge_type: EdgeType,
        weight: f32,
        confidence: f32,
    ) -> Self {
        Self {
            source,
            target,
            edge_type,
            weight,
            confidence,
            timestamp: chrono::Utc::now().timestamp(),
            inferred: false,
            inference_chain: None,
        }
    }

    /// Create an inferred edge with the inference chain
    pub fn inferred(
        source: u64,
        target: u64,
        edge_type: EdgeType,
        weight: f32,
        confidence: f32,
        chain: Vec<u64>,
    ) -> Self {
        Self {
            source,
            target,
            edge_type,
            weight,
            confidence,
            timestamp: chrono::Utc::now().timestamp(),
            inferred: true,
            inference_chain: Some(chain),
        }
    }

    /// Get the effective strength (weight * confidence * type factor)
    pub fn effective_strength(&self) -> f32 {
        self.weight * self.confidence * self.edge_type.confidence_factor()
    }

    /// Serialize edge to bytes (for storage)
    /// Format: source(8) + target(8) + type(1) + weight(4) + confidence(4) + timestamp(8) + inferred(1) + chain_len(2) + chain(8*n)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(36);

        bytes.extend_from_slice(&self.source.to_le_bytes());
        bytes.extend_from_slice(&self.target.to_le_bytes());
        bytes.push(self.edge_type.to_byte());
        bytes.extend_from_slice(&self.weight.to_le_bytes());
        bytes.extend_from_slice(&self.confidence.to_le_bytes());
        bytes.extend_from_slice(&self.timestamp.to_le_bytes());
        bytes.push(if self.inferred { 1 } else { 0 });

        if let Some(ref chain) = self.inference_chain {
            let len = chain.len().min(u16::MAX as usize) as u16;
            bytes.extend_from_slice(&len.to_le_bytes());
            for &id in chain.iter().take(len as usize) {
                bytes.extend_from_slice(&id.to_le_bytes());
            }
        } else {
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }

        bytes
    }

    /// Deserialize edge from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 36 {
            return None;
        }

        let source = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let target = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
        let edge_type = EdgeType::from_byte(bytes[16])?;
        let weight = f32::from_le_bytes(bytes[17..21].try_into().ok()?);
        let confidence = f32::from_le_bytes(bytes[21..25].try_into().ok()?);
        let timestamp = i64::from_le_bytes(bytes[25..33].try_into().ok()?);
        let inferred = bytes[33] != 0;
        let chain_len = u16::from_le_bytes(bytes[34..36].try_into().ok()?) as usize;

        let inference_chain = if chain_len > 0 && bytes.len() >= 36 + chain_len * 8 {
            let mut chain = Vec::with_capacity(chain_len);
            for i in 0..chain_len {
                let offset = 36 + i * 8;
                let id = u64::from_le_bytes(bytes[offset..offset + 8].try_into().ok()?);
                chain.push(id);
            }
            Some(chain)
        } else {
            None
        };

        Some(Self {
            source,
            target,
            edge_type,
            weight,
            confidence,
            timestamp,
            inferred,
            inference_chain,
        })
    }

    /// Get byte size for this edge
    pub fn byte_size(&self) -> usize {
        36 + self.inference_chain.as_ref().map_or(0, |c| c.len() * 8)
    }
}

// ============================================================================
// Conversion from Legacy EdgeType
// ============================================================================

use super::legacy::EdgeType as LegacyEdgeType;

/// Convert from the old EdgeType enum to the new system
pub fn from_legacy_edge_type(legacy: &LegacyEdgeType) -> EdgeType {
    match legacy {
        LegacyEdgeType::Semantic => EdgeType::Legacy(LegacyEdge::Semantic),
        LegacyEdgeType::Temporal => EdgeType::Legacy(LegacyEdge::Temporal),
        LegacyEdgeType::Manual => EdgeType::Legacy(LegacyEdge::Manual),
        LegacyEdgeType::Tag => EdgeType::Legacy(LegacyEdge::Tag),
    }
}

/// Convert to the old EdgeType enum for backward compatibility
pub fn to_legacy_edge_type(edge_type: &EdgeType) -> LegacyEdgeType {
    match edge_type {
        EdgeType::Legacy(LegacyEdge::Semantic) => LegacyEdgeType::Semantic,
        EdgeType::Legacy(LegacyEdge::Temporal) => LegacyEdgeType::Temporal,
        EdgeType::Legacy(LegacyEdge::Manual) => LegacyEdgeType::Manual,
        EdgeType::Legacy(LegacyEdge::Tag) => LegacyEdgeType::Tag,

        // Map new types to closest legacy equivalent
        EdgeType::Semantic(SemanticEdge::SimilarTo) => LegacyEdgeType::Semantic,
        EdgeType::Semantic(_) => LegacyEdgeType::Tag,
        EdgeType::Temporal(_) => LegacyEdgeType::Temporal,
        EdgeType::Structural(StructuralEdge::References) => LegacyEdgeType::Manual,
        EdgeType::Structural(_) => LegacyEdgeType::Manual,
        EdgeType::Causal(_) => LegacyEdgeType::Manual,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_type_byte_roundtrip() {
        let types = vec![
            EdgeType::Structural(StructuralEdge::References),
            EdgeType::Semantic(SemanticEdge::IsA),
            EdgeType::Causal(CausalEdge::Causes),
            EdgeType::Temporal(TemporalEdge::Before),
            EdgeType::Legacy(LegacyEdge::Semantic),
        ];

        for edge_type in types {
            let byte = edge_type.to_byte();
            let recovered = EdgeType::from_byte(byte).expect("Should parse");
            assert_eq!(edge_type, recovered);
        }
    }

    #[test]
    fn test_edge_serialization() {
        let edge = Edge::new(
            123,
            456,
            EdgeType::Causal(CausalEdge::Causes),
            0.85,
        );

        let bytes = edge.to_bytes();
        let recovered = Edge::from_bytes(&bytes).expect("Should parse");

        assert_eq!(edge.source, recovered.source);
        assert_eq!(edge.target, recovered.target);
        assert_eq!(edge.edge_type, recovered.edge_type);
        assert!((edge.weight - recovered.weight).abs() < 0.001);
    }

    #[test]
    fn test_inferred_edge_serialization() {
        let edge = Edge::inferred(
            100,
            200,
            EdgeType::Semantic(SemanticEdge::IsA),
            1.0,
            0.9,
            vec![100, 150, 200],
        );

        let bytes = edge.to_bytes();
        let recovered = Edge::from_bytes(&bytes).expect("Should parse");

        assert!(recovered.inferred);
        assert_eq!(recovered.inference_chain, Some(vec![100, 150, 200]));
    }

    #[test]
    fn test_transitive_edges() {
        assert!(EdgeType::Semantic(SemanticEdge::IsA).is_transitive());
        assert!(EdgeType::Causal(CausalEdge::Causes).is_transitive());
        assert!(EdgeType::Temporal(TemporalEdge::Before).is_transitive());

        assert!(!EdgeType::Semantic(SemanticEdge::SimilarTo).is_transitive());
        assert!(!EdgeType::Causal(CausalEdge::Contradicts).is_transitive());
    }

    #[test]
    fn test_symmetric_edges() {
        assert!(EdgeType::Semantic(SemanticEdge::SimilarTo).is_symmetric());
        assert!(EdgeType::Causal(CausalEdge::Contradicts).is_symmetric());
        assert!(EdgeType::Temporal(TemporalEdge::Concurrent).is_symmetric());

        assert!(!EdgeType::Semantic(SemanticEdge::IsA).is_symmetric());
        assert!(!EdgeType::Causal(CausalEdge::Causes).is_symmetric());
    }

    #[test]
    fn test_inverse_edges() {
        assert_eq!(
            EdgeType::Temporal(TemporalEdge::Before).inverse(),
            Some(EdgeType::Temporal(TemporalEdge::After))
        );
        assert_eq!(
            EdgeType::Temporal(TemporalEdge::After).inverse(),
            Some(EdgeType::Temporal(TemporalEdge::Before))
        );
        assert_eq!(
            EdgeType::Semantic(SemanticEdge::IsA).inverse(),
            Some(EdgeType::Semantic(SemanticEdge::InstanceOf))
        );
    }

    #[test]
    fn test_confidence_factors() {
        // High confidence types
        assert!(EdgeType::Semantic(SemanticEdge::IsA).confidence_factor() > 0.95);

        // Lower confidence for uncertain relationships
        assert!(EdgeType::Causal(CausalEdge::Causes).confidence_factor() < 0.80);

        // Temporal proximity is weakest
        assert!(EdgeType::Temporal(TemporalEdge::TemporalProximity).confidence_factor() < 0.65);
    }

    #[test]
    fn test_effective_strength() {
        let edge = Edge::with_confidence(
            1,
            2,
            EdgeType::Causal(CausalEdge::Causes),
            0.8,  // weight
            0.9,  // confidence
        );

        let strength = edge.effective_strength();
        // 0.8 * 0.9 * 0.75 (causes confidence factor) = 0.54
        assert!(strength > 0.5 && strength < 0.6);
    }
}
