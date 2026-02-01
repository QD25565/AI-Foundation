//! Engram benchmarks - comparing against SQLite baseline
//!
//! Run with: cargo bench

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use engram::Engram;
use engram::graph::{
    CsrGraph, KnowledgeGraph, QueryBuilder, QueryExecutor, Query,
    HealthChecker, EntityExtractor, Direction, EdgeFilter,
    bfs, dijkstra, find_all_paths,
};
use engram::graph::types::{Edge, EdgeType, SemanticEdge, CausalEdge};
use engram::graph::inference::{TransitiveClosure, InferenceEngine};
use rusqlite::{Connection, params};
use tempfile::TempDir;

/// Create a temporary Engram database
fn create_engram(temp_dir: &TempDir) -> Engram {
    let path = temp_dir.path().join("test.engram");
    Engram::open(&path).expect("Failed to create Engram")
}

/// Create a temporary SQLite database with equivalent schema
fn create_sqlite(temp_dir: &TempDir) -> Connection {
    let path = temp_dir.path().join("test.sqlite");
    let conn = Connection::open(&path).expect("Failed to create SQLite");

    conn.execute_batch(r#"
        CREATE TABLE notes (
            id INTEGER PRIMARY KEY,
            content TEXT NOT NULL,
            tags TEXT,
            created_at INTEGER NOT NULL,
            pinned INTEGER DEFAULT 0,
            deleted INTEGER DEFAULT 0
        );

        CREATE INDEX idx_notes_created ON notes(created_at);
        CREATE INDEX idx_notes_pinned ON notes(pinned);
    "#).expect("Failed to create schema");

    conn
}

/// Benchmark: Insert single note
fn bench_insert_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_single");

    let content = "This is a test note for benchmarking. It contains enough content to be realistic.";
    let tags = &["benchmark", "test"];

    // Engram
    group.bench_function("engram", |b| {
        let temp_dir = TempDir::new().unwrap();
        let mut db = create_engram(&temp_dir);
        b.iter(|| {
            db.remember(black_box(content), black_box(tags)).unwrap()
        });
    });

    // SQLite
    group.bench_function("sqlite", |b| {
        let temp_dir = TempDir::new().unwrap();
        let conn = create_sqlite(&temp_dir);
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        b.iter(|| {
            conn.execute(
                "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
                params![content, "benchmark,test", now],
            ).unwrap()
        });
    });

    group.finish();
}

/// Benchmark: Batch insert notes
fn bench_insert_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_batch");

    for size in [10, 100, 1000].iter() {
        let notes: Vec<String> = (0..*size)
            .map(|i| format!("Note number {} with some content for testing", i))
            .collect();

        // Engram
        group.bench_with_input(BenchmarkId::new("engram", size), size, |b, _| {
            let temp_dir = TempDir::new().unwrap();
            let mut db = create_engram(&temp_dir);
            b.iter(|| {
                for note in &notes {
                    db.remember(black_box(note), black_box(&["batch"])).unwrap();
                }
            });
        });

        // SQLite
        group.bench_with_input(BenchmarkId::new("sqlite", size), size, |b, _| {
            let temp_dir = TempDir::new().unwrap();
            let conn = create_sqlite(&temp_dir);
            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            b.iter(|| {
                for note in &notes {
                    conn.execute(
                        "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
                        params![note, "batch", now],
                    ).unwrap();
                }
            });
        });
    }

    group.finish();
}

/// Benchmark: Read by ID
fn bench_read_by_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_by_id");

    // Setup: Insert 1000 notes
    let temp_dir_engram = TempDir::new().unwrap();
    let mut engram = create_engram(&temp_dir_engram);
    let mut ids: Vec<u64> = Vec::new();
    for i in 0..1000 {
        let id = engram.remember(&format!("Note {}", i), &["test"]).unwrap();
        ids.push(id);
    }

    let temp_dir_sqlite = TempDir::new().unwrap();
    let sqlite = create_sqlite(&temp_dir_sqlite);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    for i in 0..1000 {
        sqlite.execute(
            "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
            params![format!("Note {}", i), "test", now],
        ).unwrap();
    }

    // Benchmark random reads
    group.bench_function("engram", |b| {
        let mut idx = 0;
        b.iter(|| {
            let id = ids[idx % ids.len()];
            idx += 1;
            engram.get(black_box(id)).unwrap()
        });
    });

    group.bench_function("sqlite", |b| {
        let mut idx = 0;
        b.iter(|| {
            let id = (idx % 1000) + 1;
            idx += 1;
            sqlite.query_row(
                "SELECT id, content, tags, created_at, pinned FROM notes WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            ).unwrap()
        });
    });

    group.finish();
}

/// Benchmark: List recent notes
fn bench_list_recent(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_recent");

    // Setup: Insert 1000 notes
    let temp_dir_engram = TempDir::new().unwrap();
    let mut engram = create_engram(&temp_dir_engram);
    for i in 0..1000 {
        engram.remember(&format!("Note {}", i), &["test"]).unwrap();
    }

    let temp_dir_sqlite = TempDir::new().unwrap();
    let sqlite = create_sqlite(&temp_dir_sqlite);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    for i in 0..1000 {
        sqlite.execute(
            "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
            params![format!("Note {}", i), "test", now],
        ).unwrap();
    }

    for limit in [10, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::new("engram", limit), limit, |b, &limit| {
            b.iter(|| {
                engram.list(black_box(limit)).unwrap()
            });
        });

        group.bench_with_input(BenchmarkId::new("sqlite", limit), limit, |b, &limit| {
            b.iter(|| {
                let mut stmt = sqlite.prepare_cached(
                    "SELECT id, content, tags, created_at, pinned FROM notes
                     WHERE deleted = 0 ORDER BY created_at DESC LIMIT ?1"
                ).unwrap();
                stmt.query_map(params![limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                }).unwrap().collect::<Vec<_>>()
            });
        });
    }

    group.finish();
}

/// Benchmark: Query by tag
fn bench_query_by_tag(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_by_tag");

    // Setup: Insert notes with various tags
    let temp_dir_engram = TempDir::new().unwrap();
    let mut engram = create_engram(&temp_dir_engram);
    for i in 0..1000 {
        let tags = match i % 4 {
            0 => vec!["alpha"],
            1 => vec!["beta"],
            2 => vec!["alpha", "beta"],
            _ => vec!["gamma"],
        };
        engram.remember(&format!("Note {}", i), &tags).unwrap();
    }

    let temp_dir_sqlite = TempDir::new().unwrap();
    let sqlite = create_sqlite(&temp_dir_sqlite);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    for i in 0..1000 {
        let tags = match i % 4 {
            0 => "alpha",
            1 => "beta",
            2 => "alpha,beta",
            _ => "gamma",
        };
        sqlite.execute(
            "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
            params![format!("Note {}", i), tags, now],
        ).unwrap();
    }

    // Create index for tag searches in SQLite
    sqlite.execute("CREATE INDEX idx_notes_tags ON notes(tags)", []).ok();

    group.bench_function("engram", |b| {
        b.iter(|| {
            engram.by_tag(black_box("alpha")).unwrap()
        });
    });

    group.bench_function("sqlite", |b| {
        b.iter(|| {
            let mut stmt = sqlite.prepare_cached(
                "SELECT id, content, tags, created_at, pinned FROM notes
                 WHERE tags LIKE '%alpha%' AND deleted = 0"
            ).unwrap();
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            }).unwrap().collect::<Vec<_>>()
        });
    });

    group.finish();
}

/// Benchmark: Cold start (open database)
fn bench_cold_start(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_start");

    // Setup: Create databases with 1000 notes
    let temp_dir_engram = TempDir::new().unwrap();
    let engram_path = temp_dir_engram.path().join("test.engram");
    {
        let mut db = Engram::open(&engram_path).unwrap();
        for i in 0..1000 {
            db.remember(&format!("Note {}", i), &["test"]).unwrap();
        }
    }

    // Create version WITH persisted indexes
    let temp_dir_engram_persisted = TempDir::new().unwrap();
    let engram_persisted_path = temp_dir_engram_persisted.path().join("test_persisted.engram");
    {
        let mut db = Engram::open(&engram_persisted_path).unwrap();
        for i in 0..1000 {
            db.remember(&format!("Note {}", i), &["test"]).unwrap();
        }
        db.persist_indexes().unwrap();
    }

    let temp_dir_sqlite = TempDir::new().unwrap();
    let sqlite_path = temp_dir_sqlite.path().join("test.sqlite");
    {
        let conn = Connection::open(&sqlite_path).unwrap();
        conn.execute_batch(r#"
            CREATE TABLE notes (
                id INTEGER PRIMARY KEY,
                content TEXT NOT NULL,
                tags TEXT,
                created_at INTEGER NOT NULL,
                pinned INTEGER DEFAULT 0,
                deleted INTEGER DEFAULT 0
            );
            CREATE INDEX idx_notes_created ON notes(created_at);
        "#).unwrap();
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        for i in 0..1000 {
            conn.execute(
                "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
                params![format!("Note {}", i), "test", now],
            ).unwrap();
        }
    }

    // Benchmark opening - WITHOUT persisted indexes (O(n) rebuild)
    group.bench_function("engram_rebuild", |b| {
        b.iter(|| {
            Engram::open(black_box(&engram_path)).unwrap()
        });
    });

    // Benchmark opening - WITH persisted indexes (O(1) load)
    group.bench_function("engram_persisted", |b| {
        b.iter(|| {
            Engram::open(black_box(&engram_persisted_path)).unwrap()
        });
    });

    group.bench_function("sqlite", |b| {
        b.iter(|| {
            Connection::open(black_box(&sqlite_path)).unwrap()
        });
    });

    group.finish();
}

/// Benchmark: Query by non-existent tag (Bloom filter benefit)
fn bench_negative_tag_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("negative_tag_lookup");

    // Setup: Insert notes with various tags (same as query_by_tag)
    let temp_dir_engram = TempDir::new().unwrap();
    let mut engram = create_engram(&temp_dir_engram);
    for i in 0..1000 {
        let tags = match i % 4 {
            0 => vec!["alpha"],
            1 => vec!["beta"],
            2 => vec!["alpha", "beta"],
            _ => vec!["gamma"],
        };
        engram.remember(&format!("Note {}", i), &tags).unwrap();
    }

    let temp_dir_sqlite = TempDir::new().unwrap();
    let sqlite = create_sqlite(&temp_dir_sqlite);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    for i in 0..1000 {
        let tags = match i % 4 {
            0 => "alpha",
            1 => "beta",
            2 => "alpha,beta",
            _ => "gamma",
        };
        sqlite.execute(
            "INSERT INTO notes (content, tags, created_at) VALUES (?1, ?2, ?3)",
            params![format!("Note {}", i), tags, now],
        ).unwrap();
    }
    sqlite.execute("CREATE INDEX idx_notes_tags ON notes(tags)", []).ok();

    // Query for a tag that definitely doesn't exist
    // Engram: Bloom filter says "definitely not" -> O(1)
    // SQLite: Must scan the index
    group.bench_function("engram", |b| {
        b.iter(|| {
            engram.by_tag(black_box("nonexistent_tag_xyz123")).unwrap()
        });
    });

    group.bench_function("sqlite", |b| {
        b.iter(|| {
            let mut stmt = sqlite.prepare_cached(
                "SELECT id, content, tags, created_at, pinned FROM notes
                 WHERE tags LIKE '%nonexistent_tag_xyz123%' AND deleted = 0"
            ).unwrap();
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            }).unwrap().collect::<Vec<_>>()
        });
    });

    group.finish();
}

// ============================================================================
// Knowledge Graph 2.0 Benchmarks
// ============================================================================

/// Create a test graph with N nodes and ~3N edges
fn create_test_graph(num_nodes: u64) -> CsrGraph {
    let mut graph = CsrGraph::new();

    // Create chain: 1 -> 2 -> 3 -> ... -> N
    for i in 1..num_nodes {
        graph.add_edge(Edge::new(i, i + 1, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
    }

    // Add some cross-links (every 10th node links to node 1)
    for i in (10..=num_nodes).step_by(10) {
        graph.add_edge(Edge::new(i, 1, EdgeType::Semantic(SemanticEdge::IsA), 0.8));
    }

    // Add some branches
    for i in (5..=num_nodes).step_by(5) {
        if i + 100 <= num_nodes {
            graph.add_edge(Edge::new(i, i + 100, EdgeType::Causal(CausalEdge::Causes), 0.7));
        }
    }

    graph.compact();
    graph
}

/// Benchmark: Add edge to graph
fn bench_graph_add_edge(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_add_edge");

    group.bench_function("csr_graph", |b| {
        let mut graph = CsrGraph::new();
        let mut counter = 0u64;
        b.iter(|| {
            counter += 1;
            graph.add_edge(Edge::new(
                black_box(counter),
                black_box(counter + 1),
                EdgeType::Semantic(SemanticEdge::RelatedTo),
                0.9,
            ));
        });
    });

    group.finish();
}

/// Benchmark: Get neighbors
fn bench_graph_neighbors(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_neighbors");

    let graph = create_test_graph(1000);

    group.bench_function("outgoing_1000_nodes", |b| {
        let mut idx = 1u64;
        b.iter(|| {
            let node = (idx % 1000) + 1;
            idx += 1;
            graph.outgoing_edges(black_box(node))
        });
    });

    group.bench_function("incoming_1000_nodes", |b| {
        let mut idx = 1u64;
        b.iter(|| {
            let node = (idx % 1000) + 1;
            idx += 1;
            graph.incoming_edges(black_box(node))
        });
    });

    group.finish();
}

/// Benchmark: BFS traversal
fn bench_graph_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_bfs");

    for size in [100, 500, 1000].iter() {
        let graph = create_test_graph(*size);

        group.bench_with_input(BenchmarkId::new("depth_3", size), size, |b, _| {
            b.iter(|| {
                bfs(&graph, black_box(1), 3, Direction::Outgoing, &EdgeFilter::All)
            });
        });

        group.bench_with_input(BenchmarkId::new("depth_5", size), size, |b, _| {
            b.iter(|| {
                bfs(&graph, black_box(1), 5, Direction::Outgoing, &EdgeFilter::All)
            });
        });
    }

    group.finish();
}

/// Benchmark: Dijkstra shortest path
fn bench_graph_dijkstra(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_dijkstra");

    let graph = create_test_graph(1000);

    group.bench_function("short_path", |b| {
        b.iter(|| {
            dijkstra(&graph, black_box(1), black_box(10), Direction::Outgoing, &EdgeFilter::All)
        });
    });

    group.bench_function("long_path", |b| {
        b.iter(|| {
            dijkstra(&graph, black_box(1), black_box(500), Direction::Outgoing, &EdgeFilter::All)
        });
    });

    group.finish();
}

/// Benchmark: Find all paths
fn bench_graph_all_paths(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_all_paths");

    let graph = create_test_graph(100);

    group.bench_function("max_5_paths", |b| {
        b.iter(|| {
            find_all_paths(&graph, black_box(1), black_box(50), 10, Direction::Outgoing, &EdgeFilter::All, 5)
        });
    });

    group.finish();
}

/// Benchmark: Transitive closure computation
fn bench_graph_transitive_closure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_transitive_closure");

    for size in [100, 500].iter() {
        let graph = create_test_graph(*size);

        group.bench_with_input(BenchmarkId::new("compute", size), size, |b, _| {
            b.iter(|| {
                TransitiveClosure::compute(black_box(&graph), 5)
            });
        });
    }

    group.finish();
}

/// Benchmark: Inference engine
fn bench_graph_inference(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_inference");

    // Create a graph with IsA relationships for inference
    let mut graph = CsrGraph::new();
    for i in 1..100 {
        graph.add_edge(Edge::new(i, i + 1, EdgeType::Semantic(SemanticEdge::IsA), 0.9));
    }
    graph.compact();

    group.bench_function("forward_chain_100", |b| {
        b.iter(|| {
            let mut engine = InferenceEngine::new();
            engine.run_inference(black_box(&graph))
        });
    });

    group.finish();
}

/// Benchmark: Health check
fn bench_graph_health_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_health_check");

    let graph = create_test_graph(1000);

    group.bench_function("full_check_1000", |b| {
        let checker = HealthChecker::new();
        b.iter(|| {
            checker.check(black_box(&graph))
        });
    });

    group.finish();
}

/// Benchmark: Entity extraction
fn bench_entity_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("entity_extraction");

    let extractor = EntityExtractor::new();
    let short_text = "The EntityExtractor class handles @mentions and #tags.";
    let long_text = "The EntityExtractor class handles @mentions and #tags. It can extract URLs like https://example.com and version numbers like v1.2.3. CamelCase and snake_case identifiers are also detected. API, REST, and JSON are recognized as acronyms. John Smith works at Acme Corporation.";

    group.bench_function("short_text", |b| {
        b.iter(|| {
            extractor.extract(black_box(short_text))
        });
    });

    group.bench_function("long_text", |b| {
        b.iter(|| {
            extractor.extract(black_box(long_text))
        });
    });

    group.finish();
}

/// Benchmark: Query execution
fn bench_graph_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_query");

    let graph = create_test_graph(1000);

    group.bench_function("path_query", |b| {
        let executor = QueryExecutor::new(&graph);
        b.iter(|| {
            executor.execute(Query::Path {
                source: black_box(1),
                target: black_box(100),
                max_hops: Some(20),
                edge_filter: None,
            })
        });
    });

    group.bench_function("neighbors_query", |b| {
        let executor = QueryExecutor::new(&graph);
        b.iter(|| {
            executor.execute(Query::Neighbors {
                node: black_box(50),
                edge_type: None,
                direction: Direction::Both,
                depth: 2,
            })
        });
    });

    group.bench_function("reachable_query", |b| {
        let executor = QueryExecutor::new(&graph);
        b.iter(|| {
            executor.execute(Query::Reachable {
                source: black_box(1),
                target: black_box(500),
                edge_types: None,
                max_hops: Some(10),
            })
        });
    });

    group.finish();
}

/// Benchmark: Graph serialization
fn bench_graph_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_serialization");

    let graph = create_test_graph(1000);
    let bytes = graph.to_bytes();

    group.bench_function("serialize_1000", |b| {
        b.iter(|| {
            black_box(&graph).to_bytes()
        });
    });

    group.bench_function("deserialize_1000", |b| {
        b.iter(|| {
            CsrGraph::from_bytes(black_box(&bytes))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_insert_single,
    bench_insert_batch,
    bench_read_by_id,
    bench_list_recent,
    bench_query_by_tag,
    bench_negative_tag_lookup,
    bench_cold_start,
);

criterion_group!(
    graph_benches,
    bench_graph_add_edge,
    bench_graph_neighbors,
    bench_graph_bfs,
    bench_graph_dijkstra,
    bench_graph_all_paths,
    bench_graph_transitive_closure,
    bench_graph_inference,
    bench_graph_health_check,
    bench_entity_extraction,
    bench_graph_query,
    bench_graph_serialization,
);

criterion_main!(benches, graph_benches);
