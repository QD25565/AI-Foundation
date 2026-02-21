//! TeamEngram V2 Benchmark
//!
//! Measures actual latencies for V2 components:
//! - Event log append/read
//! - View sync

use std::path::PathBuf;
use std::time::Instant;

use teamengram::event::Event;
use teamengram::event_log::{EventLogWriter, EventLogReader};
use teamengram::view::ViewEngine;

const WARMUP_ITERATIONS: usize = 100;
const BENCHMARK_ITERATIONS: usize = 10_000;

fn temp_dir() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("teamengram_bench_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&path);
    path
}

fn main() {
    println!("|V2 BENCHMARK|");
    println!("Iterations:{}", BENCHMARK_ITERATIONS);
    println!();

    let data_dir = temp_dir();

    // Benchmark 1: Event Log Append
    benchmark_event_log_append(&data_dir);

    // Benchmark 2: Event Log Read
    benchmark_event_log_read(&data_dir);

    // Benchmark 3: View Sync
    benchmark_view_sync(&data_dir);

    // Cleanup
    let _ = std::fs::remove_dir_all(&data_dir);

    println!("|COMPLETE|");
}

fn benchmark_event_log_append(data_dir: &PathBuf) {
    let log_dir = data_dir.join("log_append");
    std::fs::create_dir_all(&log_dir).unwrap();

    let mut writer = EventLogWriter::open(Some(&log_dir)).unwrap();

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        let event = Event::broadcast("ai-1", "warmup", "test");
        let _ = writer.append(&event);
    }

    // Benchmark
    let start = Instant::now();
    for i in 0..BENCHMARK_ITERATIONS {
        let event = Event::broadcast("ai-1", "general", &format!("msg{}", i));
        writer.append(&event).unwrap();
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / BENCHMARK_ITERATIONS as u128;
    let throughput = BENCHMARK_ITERATIONS as f64 / elapsed.as_secs_f64();

    println!("|EVENT LOG APPEND|");
    println!("  Total:{}ms", elapsed.as_millis());
    println!("  Average:{}ns", avg_ns);
    println!("  Throughput:{:.0}/sec", throughput);
    println!();
}

fn benchmark_event_log_read(data_dir: &PathBuf) {
    let log_dir = data_dir.join("log_read");
    std::fs::create_dir_all(&log_dir).unwrap();

    // Create log with events
    {
        let mut writer = EventLogWriter::open(Some(&log_dir)).unwrap();
        for i in 0..(WARMUP_ITERATIONS + BENCHMARK_ITERATIONS) {
            let event = Event::broadcast("ai-1", "general", &format!("msg{}", i));
            writer.append(&event).unwrap();
        }
    }

    let mut reader = EventLogReader::open(Some(&log_dir)).unwrap();

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        let _ = reader.try_read();
    }

    // Benchmark
    let start = Instant::now();
    let mut count = 0;
    while let Ok(Some(_)) = reader.try_read() {
        count += 1;
        if count >= BENCHMARK_ITERATIONS {
            break;
        }
    }
    let elapsed = start.elapsed();

    let avg_ns = if count > 0 { elapsed.as_nanos() / count as u128 } else { 0 };
    let throughput = count as f64 / elapsed.as_secs_f64();

    println!("|EVENT LOG READ|");
    println!("  Events:{}", count);
    println!("  Total:{}ms", elapsed.as_millis());
    println!("  Average:{}ns", avg_ns);
    println!("  Throughput:{:.0}/sec", throughput);
    println!();
}

fn benchmark_view_sync(data_dir: &PathBuf) {
    let view_dir = data_dir.join("view");
    let log_dir = data_dir.join("view_log");
    std::fs::create_dir_all(&view_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();

    // Create log with various event types
    {
        let mut writer = EventLogWriter::open(Some(&log_dir)).unwrap();
        for i in 0..BENCHMARK_ITERATIONS {
            let event = match i % 5 {
                0 => Event::broadcast("ai-2", "general", &format!("msg{}", i)),
                1 => Event::direct_message("ai-2", "ai-1", &format!("dm{}", i)),
                2 => Event::dialogue_start("ai-2", "ai-1", "review"),
                3 => Event::file_action("ai-1", "file.rs", "read"),
                _ => Event::broadcast("ai-3", "general", &format!("msg{}", i)),
            };
            writer.append(&event).unwrap();
        }
    }

    // Create view engine
    let mut view = ViewEngine::open("ai-1", &view_dir).unwrap();
    let mut reader = EventLogReader::open(Some(&log_dir)).unwrap();

    // Benchmark view sync
    let start = Instant::now();
    let synced = view.sync(&mut reader).unwrap();
    let elapsed = start.elapsed();

    let avg_ns = if synced > 0 {
        elapsed.as_nanos() / synced as u128
    } else {
        0
    };
    let throughput = synced as f64 / elapsed.as_secs_f64();

    println!("|VIEW SYNC|");
    println!("  Events:{}", synced);
    println!("  Total:{}ms", elapsed.as_millis());
    println!("  Average:{}ns/event", avg_ns);
    println!("  Throughput:{:.0}/sec", throughput);

    let stats = view.stats();
    println!("  UnreadDMs:{}", stats.unread_dms);
    println!("  Dialogues:{}", stats.active_dialogues);
    println!("  Tasks:{}", stats.my_tasks);
    println!();
}
