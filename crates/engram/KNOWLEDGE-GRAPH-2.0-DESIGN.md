# Engram Knowledge Graph 2.0 - Design Document

**Purpose:** Transform Engram from a simple note database into a full-fledged AI knowledge engine with inference, reasoning, and semantic understanding.

**Goal:** Create the most powerful purpose-built AI memory system on the planet.

---

## Research Summary

### Academic Foundations

| Area | Key Insight | Source |
|------|-------------|--------|
| **HybridRAG** | Combining KG + Vector search outperforms either alone | [arXiv:2408.04948](https://arxiv.org/abs/2408.04948) |
| **Transitive Closure** | O(n) amortized for insertions, O(1) queries with materialization | [Italiano et al.](https://www.sciencedirect.com/science/article/pii/S0022000002918830) |
| **KG Embeddings** | RotatE > ComplEx > TransE for relation modeling | [arXiv:1902.10197](https://arxiv.org/abs/1902.10197) |
| **Inference** | Forward chaining + materialization = fast queries, expensive updates | [GraphDB Docs](https://graphdb.ontotext.com/documentation/standard/reasoning.html) |
| **Uncertainty** | Confidence propagation via probabilistic box embeddings | [arXiv:2104.04597](https://ar5iv.labs.arxiv.org/html/2104.04597) |
| **Contradiction** | Anti-pattern detection scales better than exhaustive search | [arXiv:2502.19023](https://arxiv.org/html/2502.19023v1) |

### Industry Benchmarks

| System | Query Latency | Scale |
|--------|---------------|-------|
| Neo4j | ~100ms @ 16.7B triples | Enterprise |
| Stardog | <100ms for 92% queries | Enterprise |
| GraphDB | Varies by reasoning level | Enterprise |
| **Engram Target** | **<1ms for 99% queries** | **<10M edges** |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         ENGRAM KNOWLEDGE ENGINE 2.0                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 5: QUERY & REASONING                                          │    │
│  │ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │    │
│  │ │   Query     │ │  Reasoning  │ │ Explanation │ │   HybridRAG     │ │    │
│  │ │  Language   │ │   Chains    │ │  Generator  │ │   Interface     │ │    │
│  │ └─────────────┘ └─────────────┘ └─────────────┘ └─────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                    │                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 4: INFERENCE & CONSISTENCY                                    │    │
│  │ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │    │
│  │ │  Forward    │ │ Transitive  │ │Contradiction│ │   Confidence    │ │    │
│  │ │  Chaining   │ │  Closure    │ │  Detection  │ │  Propagation    │ │    │
│  │ └─────────────┘ └─────────────┘ └─────────────┘ └─────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                    │                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 3: SEMANTIC UNDERSTANDING                                     │    │
│  │ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │    │
│  │ │   Entity    │ │  Relation   │ │  Concept    │ │   Auto-Link     │ │    │
│  │ │ Extraction  │ │Classification│ │ Hierarchy   │ │    Engine       │ │    │
│  │ └─────────────┘ └─────────────┘ └─────────────┘ └─────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                    │                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 2: GRAPH OPERATIONS                                           │    │
│  │ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │    │
│  │ │  Multi-hop  │ │   Path      │ │  Subgraph   │ │   Community     │ │    │
│  │ │  Traversal  │ │  Finding    │ │ Extraction  │ │   Detection     │ │    │
│  │ └─────────────┘ └─────────────┘ └─────────────┘ └─────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                    │                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 1: CORE STORAGE (Current Engram)                              │    │
│  │ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │    │
│  │ │   Notes     │ │   Edges     │ │  Vectors    │ │    PageRank     │ │    │
│  │ │  (mmap)     │ │  (HashMap)  │ │   (HNSW)    │ │    (cached)     │ │    │
│  │ └─────────────┘ └─────────────┘ └─────────────┘ └─────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Edge Type Taxonomy

### Current (4 types)
```rust
enum EdgeType {
    Semantic,  // Embedding similarity
    Temporal,  // Time proximity
    Manual,    // User-created
    Tag,       // Shared tag
}
```

### Proposed (16+ types)

```rust
/// Structural relationships
enum StructuralEdge {
    References,    // Note A mentions/cites note B
    Continues,     // Note A continues thought from B
    Supersedes,    // Note A replaces/updates B
    Contains,      // Note A contains B (hierarchical)
    DerivedFrom,   // Note A was derived from B
}

/// Semantic relationships
enum SemanticEdge {
    IsA,           // Concept hierarchy (dog IsA animal)
    PartOf,        // Composition (wheel PartOf car)
    RelatedTo,     // General association
    SimilarTo,     // Semantic similarity (via embeddings)
    SynonymOf,     // Same meaning
    AntonymOf,     // Opposite meaning
}

/// Causal relationships
enum CausalEdge {
    Causes,        // A caused B
    Implies,       // If A then B (logical)
    Contradicts,   // A conflicts with B
    Supports,      // A provides evidence for B
    Prevents,      // A prevents B
    Enables,       // A enables B to happen
}

/// Temporal relationships
enum TemporalEdge {
    Before,        // A happened before B
    After,         // A happened after B
    During,        // A happened during B
    TriggeredBy,   // A was triggered by B
}

/// Meta edge for all types
struct Edge {
    source: u64,
    target: u64,
    edge_type: EdgeTypeUnion,
    weight: f32,           // 0.0 - 1.0
    confidence: f32,       // 0.0 - 1.0 (how certain)
    timestamp: i64,        // When edge was created
    inferred: bool,        // Was this inferred or explicit?
    inference_chain: Option<Vec<u64>>,  // How was it inferred?
}
```

---

## Data Structures

### 1. CSR (Compressed Sparse Row) for Graph Storage

```rust
/// Memory-efficient graph storage
/// O(V + E) space, O(1) neighbor access, cache-friendly traversal
struct CsrGraph {
    /// Row pointers: node_id -> start index in edges array
    /// Length: num_nodes + 1
    row_ptr: Vec<u32>,

    /// Column indices: target node IDs
    /// Length: num_edges
    col_idx: Vec<u32>,

    /// Edge data (parallel to col_idx)
    edge_type: Vec<u8>,
    edge_weight: Vec<f32>,
    edge_confidence: Vec<f32>,

    /// Node ID mapping (for sparse node IDs)
    node_to_idx: HashMap<u64, u32>,
    idx_to_node: Vec<u64>,
}

impl CsrGraph {
    /// Get neighbors of node - O(degree)
    fn neighbors(&self, node: u64) -> &[Edge] { ... }

    /// Check if edge exists - O(degree) with binary search
    fn has_edge(&self, from: u64, to: u64) -> bool { ... }

    /// Rebuild CSR from edge list - O(E log E)
    fn rebuild(&mut self, edges: &[Edge]) { ... }
}
```

### 2. Transitive Closure with Incremental Maintenance

```rust
/// Bit-matrix transitive closure for small graphs (<10K nodes)
/// O(1) reachability queries, O(n²) space
struct TransitiveClosure {
    /// Reachability matrix: reachable[i * n + j] = true if i can reach j
    reachable: BitVec,
    num_nodes: usize,

    /// For incremental updates
    dirty: bool,
}

impl TransitiveClosure {
    /// Can node A reach node B? - O(1)
    fn can_reach(&self, from: u64, to: u64) -> bool {
        let i = self.node_to_idx(from);
        let j = self.node_to_idx(to);
        self.reachable[i * self.num_nodes + j]
    }

    /// Add edge and update closure - O(n²) worst case, O(affected) typical
    fn add_edge(&mut self, from: u64, to: u64) {
        // Italiano's algorithm for incremental TC
        let i = self.node_to_idx(from);
        let j = self.node_to_idx(to);

        if self.reachable[i * self.num_nodes + j] {
            return; // Already reachable, no change
        }

        // Update: all nodes that can reach `from` can now reach all nodes reachable from `to`
        for x in 0..self.num_nodes {
            if self.reachable[x * self.num_nodes + i] || x == i {
                for y in 0..self.num_nodes {
                    if self.reachable[j * self.num_nodes + y] || y == j {
                        self.reachable[x * self.num_nodes + y] = true;
                    }
                }
            }
        }
    }

    /// Full recomputation - O(n³) Warshall's algorithm
    fn recompute(&mut self, graph: &CsrGraph) { ... }
}
```

### 3. Inference Engine

```rust
/// Rule-based inference engine with forward chaining
struct InferenceEngine {
    rules: Vec<InferenceRule>,
    materialized: HashSet<Edge>,  // Inferred edges
}

/// An inference rule
struct InferenceRule {
    name: &'static str,
    /// Antecedent patterns (edges that must exist)
    antecedents: Vec<EdgePattern>,
    /// Consequent (edge to infer)
    consequent: EdgePattern,
    /// Confidence multiplier
    confidence_factor: f32,
}

/// Built-in rules
impl InferenceEngine {
    fn default_rules() -> Vec<InferenceRule> {
        vec![
            // Transitivity: A IsA B, B IsA C => A IsA C
            InferenceRule {
                name: "isa_transitivity",
                antecedents: vec![
                    EdgePattern::new("?a", "IsA", "?b"),
                    EdgePattern::new("?b", "IsA", "?c"),
                ],
                consequent: EdgePattern::new("?a", "IsA", "?c"),
                confidence_factor: 0.9,
            },

            // Causal chain: A Causes B, B Causes C => A Causes C (weaker)
            InferenceRule {
                name: "causal_chain",
                antecedents: vec![
                    EdgePattern::new("?a", "Causes", "?b"),
                    EdgePattern::new("?b", "Causes", "?c"),
                ],
                consequent: EdgePattern::new("?a", "Causes", "?c"),
                confidence_factor: 0.7,
            },

            // Contradiction propagation: A Supports B, C Contradicts B => C Contradicts A
            InferenceRule {
                name: "contradiction_propagation",
                antecedents: vec![
                    EdgePattern::new("?a", "Supports", "?b"),
                    EdgePattern::new("?c", "Contradicts", "?b"),
                ],
                consequent: EdgePattern::new("?c", "Contradicts", "?a"),
                confidence_factor: 0.8,
            },

            // Symmetry: A SimilarTo B => B SimilarTo A
            InferenceRule {
                name: "similarity_symmetry",
                antecedents: vec![
                    EdgePattern::new("?a", "SimilarTo", "?b"),
                ],
                consequent: EdgePattern::new("?b", "SimilarTo", "?a"),
                confidence_factor: 1.0,
            },
        ]
    }

    /// Run forward chaining until fixpoint
    fn materialize(&mut self, graph: &CsrGraph) -> usize {
        let mut new_edges = 0;
        let mut changed = true;

        while changed {
            changed = false;
            for rule in &self.rules {
                // Find all matching antecedents
                for binding in self.find_bindings(graph, &rule.antecedents) {
                    let inferred = rule.consequent.apply(&binding);
                    if !self.materialized.contains(&inferred) {
                        self.materialized.insert(inferred);
                        new_edges += 1;
                        changed = true;
                    }
                }
            }
        }

        new_edges
    }
}
```

### 4. Entity Extraction (Lightweight)

```rust
/// Lightweight entity extraction without LLM
struct EntityExtractor {
    /// Known entity patterns (regex-based)
    patterns: Vec<EntityPattern>,

    /// Entity cache: text hash -> entities
    cache: HashMap<u64, Vec<Entity>>,
}

struct Entity {
    text: String,
    entity_type: EntityType,
    start: usize,
    end: usize,
    confidence: f32,
}

enum EntityType {
    Concept,      // General concept/noun phrase
    Person,       // Named person
    Technology,   // Tech/tool name
    Date,         // Temporal reference
    Number,       // Numeric value
    Tag,          // Explicit #tag or [tag]
    Reference,    // @mention or note reference
}

impl EntityExtractor {
    /// Extract entities from text - O(n * num_patterns)
    fn extract(&mut self, text: &str) -> Vec<Entity> {
        let hash = self.hash_text(text);
        if let Some(cached) = self.cache.get(&hash) {
            return cached.clone();
        }

        let mut entities = Vec::new();

        // 1. Extract explicit tags: #tag, [tag], @mention
        entities.extend(self.extract_tags(text));

        // 2. Extract dates: 2024-12-14, "yesterday", "last week"
        entities.extend(self.extract_dates(text));

        // 3. Extract capitalized phrases (likely proper nouns/concepts)
        entities.extend(self.extract_noun_phrases(text));

        // 4. Extract quoted strings
        entities.extend(self.extract_quoted(text));

        // 5. Extract URLs and paths
        entities.extend(self.extract_references(text));

        self.cache.insert(hash, entities.clone());
        entities
    }
}
```

### 5. Auto-Linking Engine

```rust
/// Automatic edge creation based on content analysis
struct AutoLinker {
    entity_extractor: EntityExtractor,
    embedding_threshold: f32,  // Min cosine similarity for SimilarTo edge
    temporal_window: i64,      // Seconds for Temporal edge
}

impl AutoLinker {
    /// Analyze a new note and create edges
    fn link_note(&self, note: &Note, graph: &mut KnowledgeGraph) -> Vec<Edge> {
        let mut new_edges = Vec::new();

        // 1. Extract entities
        let entities = self.entity_extractor.extract(&note.content);

        // 2. Find notes with matching entities -> References edge
        for entity in &entities {
            for matching_note in graph.notes_with_entity(entity) {
                if matching_note.id != note.id {
                    new_edges.push(Edge::new(
                        note.id,
                        matching_note.id,
                        EdgeType::References,
                        entity.confidence,
                    ));
                }
            }
        }

        // 3. Find semantically similar notes -> SimilarTo edge
        let similar = graph.vector_search(&note.embedding, 10);
        for (other_id, similarity) in similar {
            if similarity >= self.embedding_threshold {
                new_edges.push(Edge::new(
                    note.id,
                    other_id,
                    EdgeType::SimilarTo,
                    similarity,
                ));
            }
        }

        // 4. Find temporally proximate notes -> Temporal edge
        let temporal_neighbors = graph.notes_in_window(
            note.timestamp - self.temporal_window,
            note.timestamp + self.temporal_window,
        );
        for other in temporal_neighbors {
            if other.id != note.id {
                let time_diff = (note.timestamp - other.timestamp).abs() as f32;
                let weight = 1.0 - (time_diff / self.temporal_window as f32);
                new_edges.push(Edge::new(
                    note.id,
                    other.id,
                    EdgeType::Temporal,
                    weight,
                ));
            }
        }

        // 5. Shared tags -> Tag edge
        for tag in &note.tags {
            for other in graph.notes_with_tag(tag) {
                if other.id != note.id {
                    new_edges.push(Edge::new(
                        note.id,
                        other.id,
                        EdgeType::Tag,
                        1.0,
                    ));
                }
            }
        }

        new_edges
    }
}
```

### 6. Contradiction Detection

```rust
/// Detect contradictions in the knowledge graph
struct ContradictionDetector {
    /// Contradiction patterns to check
    patterns: Vec<ContradictionPattern>,
}

struct ContradictionPattern {
    name: &'static str,
    check: fn(&KnowledgeGraph, u64, u64) -> Option<Contradiction>,
}

struct Contradiction {
    node_a: u64,
    node_b: u64,
    pattern: &'static str,
    explanation: String,
    confidence: f32,
}

impl ContradictionDetector {
    fn default_patterns() -> Vec<ContradictionPattern> {
        vec![
            // Mutual contradiction: A Contradicts B and B Supports A
            ContradictionPattern {
                name: "mutual_contradiction",
                check: |g, a, b| {
                    if g.has_edge(a, b, EdgeType::Contradicts) &&
                       g.has_edge(b, a, EdgeType::Supports) {
                        Some(Contradiction {
                            node_a: a,
                            node_b: b,
                            pattern: "mutual_contradiction",
                            explanation: format!(
                                "Note {} contradicts note {} but {} also supports {}",
                                a, b, b, a
                            ),
                            confidence: 0.9,
                        })
                    } else {
                        None
                    }
                },
            },

            // Circular causation: A Causes B Causes A
            ContradictionPattern {
                name: "circular_causation",
                check: |g, a, _| {
                    // Check if A can reach itself via Causes edges
                    if let Some(path) = g.find_path(a, a, EdgeType::Causes) {
                        if path.len() > 1 {
                            return Some(Contradiction {
                                node_a: a,
                                node_b: a,
                                pattern: "circular_causation",
                                explanation: format!(
                                    "Circular causation detected: {:?}",
                                    path
                                ),
                                confidence: 0.95,
                            });
                        }
                    }
                    None
                },
            },
        ]
    }

    /// Scan for all contradictions - O(E * patterns)
    fn detect_all(&self, graph: &KnowledgeGraph) -> Vec<Contradiction> {
        let mut contradictions = Vec::new();

        for pattern in &self.patterns {
            for edge in graph.all_edges() {
                if let Some(c) = (pattern.check)(graph, edge.source, edge.target) {
                    contradictions.push(c);
                }
            }
        }

        contradictions
    }
}
```

### 7. Confidence Propagation

```rust
/// Propagate confidence through inference chains
struct ConfidencePropagator {
    /// Decay factor per hop
    hop_decay: f32,  // e.g., 0.9 = 10% decay per hop

    /// Minimum confidence threshold
    min_confidence: f32,
}

impl ConfidencePropagator {
    /// Calculate confidence of an inferred fact
    fn calculate_confidence(&self, chain: &[Edge]) -> f32 {
        let mut confidence = 1.0;

        for (i, edge) in chain.iter().enumerate() {
            // Each hop decays confidence
            confidence *= edge.confidence * self.hop_decay.powi(i as i32);

            // Certain edge types reduce confidence more
            let type_factor = match edge.edge_type {
                EdgeType::Causes => 0.9,      // Causal claims are uncertain
                EdgeType::Implies => 0.95,    // Logical implications more certain
                EdgeType::SimilarTo => 0.85,  // Similarity is fuzzy
                EdgeType::IsA => 1.0,         // Hierarchy is certain
                _ => 0.9,
            };
            confidence *= type_factor;
        }

        confidence.max(self.min_confidence)
    }

    /// Update all confidence scores after graph changes
    fn propagate(&self, graph: &mut KnowledgeGraph) {
        // BFS from all explicit (non-inferred) edges
        let explicit: Vec<Edge> = graph.all_edges()
            .filter(|e| !e.inferred)
            .cloned()
            .collect();

        for edge in explicit {
            self.propagate_from(graph, &edge);
        }
    }
}
```

### 8. Query Language (Simple)

```rust
/// Simple query language for knowledge graph
///
/// Examples:
///   "path from 123 to 456"
///   "neighbors of 123 type Causes"
///   "infer 123 Implies ?"
///   "contradictions"
///   "explain 123 -> 456"

enum Query {
    /// Find path between two nodes
    Path { from: u64, to: u64, edge_types: Option<Vec<EdgeType>> },

    /// Get neighbors of a node
    Neighbors { node: u64, edge_type: Option<EdgeType>, direction: Direction },

    /// Find nodes that match a pattern
    Pattern { subject: NodePattern, predicate: EdgeType, object: NodePattern },

    /// Explain how two nodes are related
    Explain { from: u64, to: u64 },

    /// Find all contradictions
    Contradictions,

    /// Get inference chain for an edge
    InferenceChain { from: u64, to: u64 },

    /// Subgraph around a node
    Subgraph { center: u64, hops: usize },
}

impl KnowledgeGraph {
    fn query(&self, q: Query) -> QueryResult {
        match q {
            Query::Path { from, to, edge_types } => {
                let path = self.find_path(from, to, edge_types);
                QueryResult::Path(path)
            }
            Query::Neighbors { node, edge_type, direction } => {
                let neighbors = self.get_neighbors(node, edge_type, direction);
                QueryResult::Nodes(neighbors)
            }
            Query::Explain { from, to } => {
                let explanation = self.explain_relationship(from, to);
                QueryResult::Explanation(explanation)
            }
            // ... etc
        }
    }
}
```

---

## Performance Targets

| Operation | Current | Target | Method |
|-----------|---------|--------|--------|
| Add edge | ~1µs | <500ns | Pre-allocated vectors, no realloc |
| Get neighbors | O(1) HashMap | O(1) CSR | Cache-friendly memory layout |
| Check reachability | O(V+E) BFS | O(1) | Transitive closure matrix |
| Multi-hop (3 hops) | ~100µs | <10µs | CSR + SIMD neighbor iteration |
| Inference (full) | N/A | <100ms | Incremental forward chaining |
| Find contradictions | N/A | <10ms | Pre-computed contradiction index |
| Confidence update | N/A | <1ms | Lazy propagation |

---

## Implementation Phases

### Phase 1: Extended Edge Types + Typed Storage (Week 1)
- [ ] Define full EdgeType enum with all 16+ types
- [ ] Update Edge struct with confidence, timestamp, inferred flag
- [ ] Implement CSR graph storage
- [ ] Add bidirectional edge support
- [ ] Benchmark vs current HashMap implementation

### Phase 2: Multi-hop Traversal + Path Finding (Week 1-2)
- [ ] Implement BFS/DFS with edge type filtering
- [ ] Add Dijkstra for weighted shortest path
- [ ] Implement A* with heuristic for large graphs
- [ ] Add path caching for frequent queries
- [ ] Benchmark path finding performance

### Phase 3: Transitive Closure + Inference Engine (Week 2)
- [ ] Implement bit-matrix transitive closure
- [ ] Add Italiano's incremental maintenance
- [ ] Define inference rules (IsA transitivity, causal chains, etc.)
- [ ] Implement forward chaining materialization
- [ ] Add rule priority and conflict resolution

### Phase 4: Entity Extraction + Auto-linking (Week 2-3)
- [ ] Build regex-based entity extractor
- [ ] Implement concept/noun phrase extraction
- [ ] Create auto-linker for new notes
- [ ] Add entity index for fast lookup
- [ ] Integrate with remember() operation

### Phase 5: Contradiction Detection + Confidence (Week 3)
- [ ] Define contradiction patterns
- [ ] Implement contradiction detector
- [ ] Add confidence propagation
- [ ] Create contradiction index for fast queries
- [ ] Add "health check" for knowledge graph

### Phase 6: Query Language + Reasoning Chains (Week 3-4)
- [ ] Design query language syntax
- [ ] Implement query parser
- [ ] Add query execution engine
- [ ] Create explanation generator
- [ ] Add CLI commands for queries

### Phase 7: Benchmarking & Verification (Week 4)
- [ ] Create comprehensive benchmark suite
- [ ] Compare with Neo4j, SQLite baseline
- [ ] Verify all features work correctly
- [ ] Optimize hot paths
- [ ] Document performance characteristics

---

## File Structure

```
engram/src/
├── lib.rs                 # Public API
├── storage.rs             # Core storage (existing)
├── graph/
│   ├── mod.rs             # Graph module
│   ├── types.rs           # Edge types, Node types
│   ├── csr.rs             # CSR storage
│   ├── traversal.rs       # BFS, DFS, path finding
│   └── transitive.rs      # Transitive closure
├── inference/
│   ├── mod.rs             # Inference module
│   ├── rules.rs           # Inference rules
│   ├── engine.rs          # Forward chaining engine
│   └── confidence.rs      # Confidence propagation
├── semantic/
│   ├── mod.rs             # Semantic module
│   ├── entity.rs          # Entity extraction
│   ├── autolink.rs        # Auto-linking
│   └── contradiction.rs   # Contradiction detection
├── query/
│   ├── mod.rs             # Query module
│   ├── language.rs        # Query language
│   ├── parser.rs          # Query parser
│   ├── executor.rs        # Query execution
│   └── explain.rs         # Explanation generator
└── bench/
    ├── mod.rs             # Benchmarks
    ├── graph_bench.rs     # Graph operation benchmarks
    └── inference_bench.rs # Inference benchmarks
```

---

## Integration with Existing Engram

The Knowledge Graph 2.0 will integrate seamlessly with existing Engram:

```rust
impl Engram {
    /// Remember a note with auto-linking
    pub fn remember_smart(&mut self, content: &str, tags: &[&str]) -> Result<u64> {
        // 1. Create note (existing)
        let note_id = self.remember(content, tags)?;

        // 2. Extract entities
        let entities = self.entity_extractor.extract(content);

        // 3. Create embeddings (existing)
        if let Some(embedding) = self.generate_embedding(content) {
            self.vector_store.insert(note_id, &embedding);
        }

        // 4. Auto-link to related notes
        let new_edges = self.auto_linker.link_note(
            &self.get_note(note_id)?,
            &self.knowledge_graph,
        );

        for edge in new_edges {
            self.knowledge_graph.add_edge(edge);
        }

        // 5. Run incremental inference
        self.inference_engine.materialize_incremental(&mut self.knowledge_graph);

        // 6. Check for contradictions
        let contradictions = self.contradiction_detector.check_node(
            note_id,
            &self.knowledge_graph,
        );
        if !contradictions.is_empty() {
            eprintln!("Warning: {} potential contradictions detected", contradictions.len());
        }

        Ok(note_id)
    }

    /// Smart recall with graph-enhanced ranking
    pub fn recall_smart(&self, query: &str, limit: usize) -> Result<Vec<RecallResult>> {
        // 1. Vector search (existing)
        let vector_results = self.vector_search(query, limit * 2);

        // 2. Keyword search (existing)
        let keyword_results = self.keyword_search(query, limit * 2);

        // 3. Graph expansion: find related via edges
        let mut expanded = HashSet::new();
        for result in vector_results.iter().chain(keyword_results.iter()) {
            // Add 1-hop neighbors
            for neighbor in self.knowledge_graph.neighbors(result.id) {
                expanded.insert(neighbor.target);
            }
        }

        // 4. HybridRAG fusion: combine all signals
        let mut scores: HashMap<u64, f32> = HashMap::new();

        for result in vector_results {
            *scores.entry(result.id).or_default() += result.score * 0.4;
        }
        for result in keyword_results {
            *scores.entry(result.id).or_default() += result.score * 0.3;
        }
        for &id in &expanded {
            let pagerank = self.knowledge_graph.get_pagerank(id);
            *scores.entry(id).or_default() += pagerank * 0.2;
        }

        // 5. Sort and return top results
        let mut results: Vec<_> = scores.into_iter().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        results.truncate(limit);

        results.into_iter()
            .map(|(id, score)| self.build_recall_result(id, score))
            .collect()
    }
}
```

---

## Sources & References

### Academic Papers
- [HybridRAG: Integrating KGs and Vector RAG](https://arxiv.org/abs/2408.04948)
- [RotatE: KG Embedding by Relational Rotation](https://arxiv.org/abs/1902.10197)
- [Fully Dynamic Algorithm for Transitive Closure](https://www.sciencedirect.com/science/article/pii/S0022000002918830)
- [Probabilistic Box Embeddings for Uncertain KG Reasoning](https://ar5iv.labs.arxiv.org/html/2104.04597)
- [Dealing with Inconsistency in Knowledge Graphs](https://arxiv.org/html/2502.19023v1)
- [KG Inference Algorithms (Stanford)](https://web.stanford.edu/class/cs520/2020/notes/What_Are_Some_Inference_Algorithms.html)

### Industry Resources
- [Neo4j Cypher Query Tuning](https://neo4j.com/docs/cypher-manual/current/query-tuning/)
- [GraphDB Reasoning Documentation](https://graphdb.ontotext.com/documentation/standard/reasoning.html)
- [Apache Jena Inference Support](https://jena.apache.org/documentation/inference/index.html)
- [Petgraph Rust Library](https://github.com/petgraph/petgraph)

### Benchmarks
- [Stardog Performance Benchmarks](https://www.stardog.com/blog/query-knowledge-graphs-with-billions-of-triples-in-less-time-than-it-takes-to-read-this-headline-about-performance-benchmarks/)
- [CSR Graph Representation](https://www.usenix.org/system/files/login/articles/login_winter20_16_kelly.pdf)

---

*Created: 2024-12-14*
*Author: Lyra (ai-2)*
*For: AI-Foundation*
