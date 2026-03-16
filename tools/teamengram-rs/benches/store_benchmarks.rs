//! Benchmarks for TeamEngram store operations.
//!
//! Compares old (full-scan) vs new (prefix-seek) query approaches,
//! and measures event compression throughput.
//!
//! Run: cargo bench --bench store_benchmarks

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use teamengram::btree::BTree;
use teamengram::event::{BroadcastPayload, EventPayload, FLAG_COMPRESSED};
use teamengram::shadow::ShadowAllocator;
use tempfile::tempdir;

/// Populate a B+Tree with `n` entries across multiple prefixes.
/// Distribution: 40% dm:, 25% bc:, 15% dg:, 10% tk:, 5% pr:, 5% rm:
fn populate_tree(alloc: &mut ShadowAllocator, n: usize) {
    let mut tree = BTree::new(alloc);
    let prefixes = [
        ("dm:", 40),
        ("bc:", 25),
        ("dg:", 15),
        ("tk:", 10),
        ("pr:", 5),
        ("rm:", 5),
    ];

    let mut id = 0u64;
    for (prefix, pct) in &prefixes {
        let count = n * pct / 100;
        for _ in 0..count {
            let key = format!("{}id:{:08}", prefix, id);
            let value = format!("{{\"id\":{},\"data\":\"benchmark payload for {} entry\"}}", id, prefix);
            tree.insert(key.as_bytes(), value.as_bytes()).unwrap();
            id += 1;
        }
    }
}

// ─── PREFIX QUERY BENCHMARKS ─────────────────────────────────────────────────

/// OLD approach: full tree scan + filter (how query_by_prefix worked before)
fn full_scan_prefix(alloc: &mut ShadowAllocator, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let tree = BTree::new(alloc);
    let mut results = Vec::new();
    let mut iter = tree.iter().unwrap();
    while let Some((key, value)) = iter.next().unwrap() {
        if key.starts_with(prefix) {
            results.push((key, value));
        }
    }
    results
}

/// NEW approach: prefix seek + iterate (current implementation)
fn prefix_seek(alloc: &mut ShadowAllocator, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let tree = BTree::new(alloc);
    let mut results = Vec::new();
    let mut iter = tree.prefix_iter(prefix).unwrap();
    while let Some((key, value)) = iter.next().unwrap() {
        results.push((key, value));
    }
    results
}

fn bench_prefix_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefix_query");

    for size in [100, 1_000, 10_000] {
        let prefix: &[u8] = b"dm:";
        let expected_matches = size * 40 / 100;

        // Each benchmark gets its own allocator to avoid borrow conflicts.
        // Data is deterministic (same populate_tree), so results are comparable.
        let dir_full = tempdir().unwrap();
        let path_full = dir_full.path().join("bench_full.teamengram");
        let mut alloc_full = ShadowAllocator::open(&path_full).unwrap();
        populate_tree(&mut alloc_full, size);

        let dir_seek = tempdir().unwrap();
        let path_seek = dir_seek.path().join("bench_seek.teamengram");
        let mut alloc_seek = ShadowAllocator::open(&path_seek).unwrap();
        populate_tree(&mut alloc_seek, size);

        // Verify both approaches return the same count
        let full_count = full_scan_prefix(&mut alloc_full, prefix).len();
        let seek_count = prefix_seek(&mut alloc_seek, prefix).len();
        assert_eq!(
            full_count, seek_count,
            "Result count mismatch at size={}: full_scan={}, prefix_seek={}",
            size, full_count, seek_count
        );
        assert_eq!(full_count, expected_matches);

        group.bench_with_input(
            BenchmarkId::new("full_scan", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(full_scan_prefix(&mut alloc_full, prefix));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("prefix_seek", size),
            &size,
            |b, _| {
                b.iter(|| {
                    black_box(prefix_seek(&mut alloc_seek, prefix));
                });
            },
        );
    }

    group.finish();
}

// ─── COMPRESSION BENCHMARKS ─────────────────────────────────────────────────

fn bench_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_compression");

    let small_payload = EventPayload::Broadcast(BroadcastPayload {
        channel: "general".into(),
        content: "Hello world".into(),
    });

    let medium_content = "A".repeat(800);
    let medium_payload = EventPayload::Broadcast(BroadcastPayload {
        channel: "general".into(),
        content: medium_content,
    });

    let large_content = "The quick brown fox jumps over the lazy dog. ".repeat(100);
    let large_payload = EventPayload::Broadcast(BroadcastPayload {
        channel: "general".into(),
        content: large_content,
    });

    for (label, payload) in [
        ("small_50B", &small_payload),
        ("medium_800B", &medium_payload),
        ("large_4KB", &large_payload),
    ] {
        let raw = payload.to_bytes();
        let raw_size = raw.len();

        group.bench_function(format!("compress/{} ({}B)", label, raw_size), |b| {
            b.iter(|| {
                black_box(payload.to_bytes_compressed());
            });
        });

        let (compressed, did_compress) = payload.to_bytes_compressed();
        if did_compress {
            let compressed_size = compressed.len();
            let ratio = (1.0 - compressed_size as f64 / raw_size as f64) * 100.0;
            group.bench_function(
                format!(
                    "decompress/{} ({}B->{}B, {:.0}% saved)",
                    label, raw_size, compressed_size, ratio
                ),
                |b| {
                    b.iter(|| {
                        black_box(EventPayload::from_bytes_with_flags(
                            &compressed,
                            FLAG_COMPRESSED,
                        ));
                    });
                },
            );
        }
    }

    group.finish();
}

// ─── SINGLE-KEY LOOKUP BENCHMARK ────────────────────────────────────────────

fn bench_point_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("point_lookup");

    for size in [100, 1_000, 10_000] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bench.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();
        populate_tree(&mut alloc, size);

        let target_key = format!("dm:id:{:08}", size / 4);

        group.bench_with_input(BenchmarkId::new("get", size), &size, |b, _| {
            let tree = BTree::new(&mut alloc);
            b.iter(|| {
                black_box(tree.get(target_key.as_bytes()).unwrap());
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_prefix_query, bench_compression, bench_point_lookup);
criterion_main!(benches);
