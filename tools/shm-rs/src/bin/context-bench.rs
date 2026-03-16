//! Context Fingerprint SHM Benchmark
//!
//! Measures latency of the seqlock-protected context fingerprint shared memory.
//! Validates performance claims: ~50-100ns read, ~200ns write.

use std::time::Instant;
use shm_rs::context::{ContextWriter, ContextReader};

const WARMUP: usize = 10_000;
const ITERATIONS: usize = 100_000;

fn main() {
    println!("===============================================================");
    println!("          CONTEXT FINGERPRINT SHM BENCHMARK");
    println!("===============================================================");
    println!();
    println!("  Layout: 64 bytes (1 cache line), seqlock-protected");
    println!("  Iterations: {} per test (warmup: {})", ITERATIONS, WARMUP);
    println!();

    let ai_id = "bench-ctx-test";

    // Create writer (creates the SHM file)
    let mut writer = ContextWriter::open_or_create(ai_id)
        .expect("Failed to create context writer");

    // Create reader from the same file
    let reader = ContextReader::open(ai_id)
        .expect("Failed to open context reader")
        .expect("Context SHM should exist after writer creates it");

    println!("---------------------------------------------------------------");
    println!("  1. WRITE LATENCY (ContextWriter::update)");
    println!("     Includes: seqlock acquire/release + 4 volatile writes + flush");
    println!("---------------------------------------------------------------");
    println!();

    bench_write(&mut writer);

    println!("---------------------------------------------------------------");
    println!("  2. READ LATENCY (ContextReader::read)");
    println!("     Includes: 2 atomic loads + 4 volatile reads (seqlock)");
    println!("---------------------------------------------------------------");
    println!();

    bench_read(&reader);

    println!("---------------------------------------------------------------");
    println!("  3. INCREMENT TOOL CALLS (ContextWriter::increment_tool_calls)");
    println!("     Includes: 1 volatile read + 1 volatile write + flush");
    println!("---------------------------------------------------------------");
    println!();

    bench_increment(&mut writer);

    println!("---------------------------------------------------------------");
    println!("  4. STALENESS CHECK (ContextReader::is_stale)");
    println!("     Includes: 1 volatile read + time comparison");
    println!("---------------------------------------------------------------");
    println!();

    bench_staleness(&reader);

    println!("---------------------------------------------------------------");
    println!("  5. WRITE-THEN-READ ROUND TRIP");
    println!("     Includes: full write + full read (simulates hook path)");
    println!("---------------------------------------------------------------");
    println!();

    bench_round_trip(&mut writer, &reader);

    println!("---------------------------------------------------------------");
    println!("  6. CONCURRENT READ UNDER WRITE CONTENTION");
    println!("     Writer thread + reader thread simultaneously");
    println!("---------------------------------------------------------------");
    println!();

    bench_contention(ai_id);

    // Cleanup
    if let Ok(path) = shm_rs::context::context_shm_path(ai_id) {
        let _ = std::fs::remove_file(path);
    }

    println!("===============================================================");
    println!("                    BENCHMARK COMPLETE");
    println!("===============================================================");
}

fn bench_write(writer: &mut ContextWriter) {
    // Warmup
    for i in 0..WARMUP as u64 {
        writer.update(i, i * 3).expect("warmup write");
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);

    for i in 0..ITERATIONS as u64 {
        let start = Instant::now();
        writer.update(i, i.wrapping_mul(0xDEAD)).expect("write");
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    print_stats("Write", &mut latencies);
}

fn bench_read(reader: &ContextReader) {
    // Warmup
    for _ in 0..WARMUP {
        std::hint::black_box(reader.read());
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let ctx = reader.read();
        std::hint::black_box(&ctx);
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    print_stats("Read", &mut latencies);
}

fn bench_increment(writer: &mut ContextWriter) {
    // Warmup
    for _ in 0..WARMUP {
        writer.increment_tool_calls().expect("warmup inc");
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        writer.increment_tool_calls().expect("inc");
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    print_stats("Increment", &mut latencies);
}

fn bench_staleness(reader: &ContextReader) {
    // Warmup
    for _ in 0..WARMUP {
        std::hint::black_box(reader.is_stale(60_000));
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let stale = reader.is_stale(60_000);
        std::hint::black_box(&stale);
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    print_stats("Staleness", &mut latencies);
}

fn bench_round_trip(writer: &mut ContextWriter, reader: &ContextReader) {
    // Warmup
    for i in 0..WARMUP as u64 {
        writer.update(i, i).expect("warmup");
        std::hint::black_box(reader.read());
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);

    for i in 0..ITERATIONS as u64 {
        let start = Instant::now();
        writer.update(i, i.wrapping_mul(7)).expect("write");
        let ctx = reader.read().expect("read");
        std::hint::black_box(&ctx);
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    print_stats("RoundTrip", &mut latencies);
}

fn bench_contention(ai_id: &str) {
    let ai_id_owned = ai_id.to_string();

    // Writer thread: continuous updates
    let writer_handle = std::thread::spawn(move || {
        let mut writer = ContextWriter::open_or_create(&ai_id_owned)
            .expect("writer open");

        // Warmup
        for i in 0..WARMUP as u64 {
            writer.update(i, i.wrapping_mul(3)).expect("warmup");
        }

        let start = Instant::now();
        for i in 0..ITERATIONS as u64 {
            writer.update(i, i.wrapping_mul(3)).expect("write");
        }
        let elapsed = start.elapsed();

        (elapsed, ITERATIONS)
    });

    // Give writer a head start
    std::thread::sleep(std::time::Duration::from_millis(1));

    // Reader thread: measure read latency under contention
    let reader = ContextReader::open(ai_id)
        .expect("reader open")
        .expect("SHM exists");

    let mut latencies = Vec::with_capacity(ITERATIONS);
    let mut torn_retries = 0u64;
    let mut successful_reads = 0u64;

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        match reader.read() {
            Some(ctx) => {
                latencies.push(start.elapsed().as_nanos() as u64);
                successful_reads += 1;
                // Verify invariant: bloom == simhash * 3
                if ctx.simhash > 0 {
                    assert_eq!(
                        ctx.bloom,
                        ctx.simhash.wrapping_mul(3),
                        "DATA CORRUPTION: simhash={}, bloom={} (expected {})",
                        ctx.simhash, ctx.bloom, ctx.simhash.wrapping_mul(3)
                    );
                }
            }
            None => {
                torn_retries += 1;
            }
        }
    }

    let (writer_elapsed, writer_count) = writer_handle.join().expect("writer panic");

    if !latencies.is_empty() {
        print_stats("Contended Read", &mut latencies);
    }

    let writer_avg_ns = writer_elapsed.as_nanos() / writer_count as u128;
    println!("    Writer avg: {}ns ({} writes in {:.1}ms)",
        writer_avg_ns, writer_count, writer_elapsed.as_secs_f64() * 1000.0);
    println!("    Successful reads: {} / {} ({:.1}%)",
        successful_reads, ITERATIONS,
        successful_reads as f64 / ITERATIONS as f64 * 100.0);
    if torn_retries > 0 {
        println!("    Torn read retries: {} ({:.3}%)",
            torn_retries, torn_retries as f64 / ITERATIONS as f64 * 100.0);
    } else {
        println!("    Torn read retries: 0 (zero contention observed)");
    }
    println!();
}

fn print_stats(label: &str, latencies: &mut [u64]) {
    if latencies.is_empty() {
        println!("    {} — no data", label);
        println!();
        return;
    }

    latencies.sort();

    let len = latencies.len();
    let avg = latencies.iter().sum::<u64>() / len as u64;
    let p50 = latencies[len / 2];
    let p99 = latencies[len * 99 / 100];
    let p999 = latencies[len * 999 / 1000];
    let min = latencies[0];
    let max = latencies[len - 1];

    println!("    {} ({} iterations):", label, len);
    println!("    +----------+-----------+");
    println!("    | Avg      | {:>7}ns |", avg);
    println!("    | P50      | {:>7}ns |", p50);
    println!("    | P99      | {:>7}ns |", p99);
    println!("    | P99.9    | {:>7}ns |", p999);
    println!("    | Min      | {:>7}ns |", min);
    println!("    | Max      | {:>7}ns |", max);
    println!("    +----------+-----------+");
    println!();
}
