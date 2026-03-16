//! Shared Memory IPC Benchmark
//!
//! Measures latency and throughput of the lock-free shared memory system.
//! Includes comparison between custom wire format and rkyv zero-copy serialization.

use std::time::Instant;
use shm_rs::{SharedRegion, Message, MessageType, ZcMessage, ZcMessageType, access_message};

const ITERATIONS: usize = 100_000;
const MESSAGE_SIZES: &[usize] = &[64, 256, 1024, 4096];

fn main() {
    println!("═══════════════════════════════════════════════════════════════");
    println!("              SHARED MEMORY IPC BENCHMARK");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // Create region with 256KB per mailbox
    let mut region = SharedRegion::open(None, 256 * 1024)
        .expect("Failed to create shared region");

    let stats = region.stats();
    println!("Region: {}", stats);
    println!();

    // Register test mailboxes
    region.register_mailbox("bench-sender").expect("Failed to register sender");
    region.register_mailbox("bench-receiver").expect("Failed to register receiver");

    println!("───────────────────────────────────────────────────────────────");
    println!("                    LATENCY BENCHMARK");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    for &size in MESSAGE_SIZES {
        benchmark_latency(&mut region, size);
    }

    println!("───────────────────────────────────────────────────────────────");
    println!("                   THROUGHPUT BENCHMARK");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    for &size in MESSAGE_SIZES {
        benchmark_throughput(&mut region, size);
    }

    // Cleanup
    region.deregister_mailbox("bench-sender");
    region.deregister_mailbox("bench-receiver");

    println!("───────────────────────────────────────────────────────────────");
    println!("              SERIALIZATION COMPARISON");
    println!("───────────────────────────────────────────────────────────────");
    println!();

    benchmark_serialization();

    println!("═══════════════════════════════════════════════════════════════");
    println!("                    BENCHMARK COMPLETE");
    println!("═══════════════════════════════════════════════════════════════");
}

fn benchmark_serialization() {
    const SERIAL_ITERATIONS: usize = 100_000;

    for &size in MESSAGE_SIZES {
        let payload = vec![0x42u8; size];

        // Custom wire format
        let custom_msg = Message::new(MessageType::DirectMessage, "bench-sender", payload.clone());

        let start = Instant::now();
        for _ in 0..SERIAL_ITERATIONS {
            let bytes = custom_msg.to_bytes();
            std::hint::black_box(&bytes);
        }
        let custom_serialize = start.elapsed();

        let custom_bytes = custom_msg.to_bytes();
        let start = Instant::now();
        for _ in 0..SERIAL_ITERATIONS {
            let msg = Message::from_bytes(&custom_bytes);
            std::hint::black_box(&msg);
        }
        let custom_deserialize = start.elapsed();

        // rkyv zero-copy
        let zc_msg = ZcMessage::new(ZcMessageType::DirectMessage, "bench-sender", payload.clone());

        let start = Instant::now();
        for _ in 0..SERIAL_ITERATIONS {
            let bytes = zc_msg.to_bytes();
            std::hint::black_box(&bytes);
        }
        let rkyv_serialize = start.elapsed();

        let rkyv_bytes = zc_msg.to_bytes();

        // Full deserialization
        let start = Instant::now();
        for _ in 0..SERIAL_ITERATIONS {
            let msg = ZcMessage::from_bytes(&rkyv_bytes);
            std::hint::black_box(&msg);
        }
        let rkyv_deserialize = start.elapsed();

        // Zero-copy access (no deserialization!)
        let start = Instant::now();
        for _ in 0..SERIAL_ITERATIONS {
            let archived = access_message(&rkyv_bytes);
            std::hint::black_box(&archived);
        }
        let rkyv_zerocopy = start.elapsed();

        let custom_ser_ns = custom_serialize.as_nanos() / SERIAL_ITERATIONS as u128;
        let custom_de_ns = custom_deserialize.as_nanos() / SERIAL_ITERATIONS as u128;
        let rkyv_ser_ns = rkyv_serialize.as_nanos() / SERIAL_ITERATIONS as u128;
        let rkyv_de_ns = rkyv_deserialize.as_nanos() / SERIAL_ITERATIONS as u128;
        let rkyv_zc_ns = rkyv_zerocopy.as_nanos() / SERIAL_ITERATIONS as u128;

        println!("  Payload size: {} bytes", size);
        println!("    ┌───────────────┬──────────────┬──────────────┬──────────────┐");
        println!("    │ Operation     │ Custom (ns)  │ rkyv (ns)    │ Speedup      │");
        println!("    ├───────────────┼──────────────┼──────────────┼──────────────┤");
        println!("    │ Serialize     │ {:>10}   │ {:>10}   │ {:>10.1}x  │",
            custom_ser_ns, rkyv_ser_ns,
            custom_ser_ns as f64 / rkyv_ser_ns.max(1) as f64);
        println!("    │ Deserialize   │ {:>10}   │ {:>10}   │ {:>10.1}x  │",
            custom_de_ns, rkyv_de_ns,
            custom_de_ns as f64 / rkyv_de_ns.max(1) as f64);
        println!("    │ Zero-copy     │ {:>10}   │ {:>10}   │ {:>10.1}x  │",
            custom_de_ns, rkyv_zc_ns,
            custom_de_ns as f64 / rkyv_zc_ns.max(1) as f64);
        println!("    └───────────────┴──────────────┴──────────────┴──────────────┘");
        println!();

        // Size comparison
        println!("    Wire sizes: Custom={} bytes, rkyv={} bytes",
            custom_bytes.len(), rkyv_bytes.len());
        println!();
    }
}

fn benchmark_latency(region: &mut SharedRegion, msg_size: usize) {
    let payload = vec![0x42u8; msg_size];
    let mut receiver = region.get_mailbox("bench-receiver").unwrap();

    // Warm up
    for _ in 0..1000 {
        let msg = Message::new(MessageType::Ping, "bench-sender", payload.clone());
        receiver.send(&msg).ok();
        receiver.receive();
    }

    // Measure
    let mut latencies = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let msg = Message::new(MessageType::Ping, "bench-sender", payload.clone());

        let start = Instant::now();
        receiver.send(&msg).expect("Send failed");
        let _ = receiver.receive().expect("Receive failed");
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    latencies.sort();

    let avg = latencies.iter().sum::<u64>() / latencies.len() as u64;
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[latencies.len() * 99 / 100];
    let min = latencies[0];
    let max = latencies[latencies.len() - 1];

    println!("  Message size: {} bytes", msg_size);
    println!("    │ Metric │ Value     │");
    println!("    ├────────┼───────────┤");
    println!("    │ Avg    │ {:>7}ns │", avg);
    println!("    │ P50    │ {:>7}ns │", p50);
    println!("    │ P99    │ {:>7}ns │", p99);
    println!("    │ Min    │ {:>7}ns │", min);
    println!("    │ Max    │ {:>7}ns │", max);
    println!();
}

fn benchmark_throughput(region: &mut SharedRegion, msg_size: usize) {
    let payload = vec![0x42u8; msg_size];
    let mut receiver = region.get_mailbox("bench-receiver").unwrap();

    // Send phase
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let msg = Message::new(MessageType::Ping, "bench-sender", payload.clone());
        while receiver.send(&msg).is_err() {
            // Drain some messages if full
            for _ in 0..100 {
                receiver.receive();
            }
        }
    }
    let send_time = start.elapsed();

    // Drain remaining
    while receiver.receive().is_some() {}

    let msgs_per_sec = ITERATIONS as f64 / send_time.as_secs_f64();
    let bytes_per_sec = (ITERATIONS * msg_size) as f64 / send_time.as_secs_f64();

    println!("  Message size: {} bytes", msg_size);
    println!("    Throughput: {:.0} msg/sec ({:.1} MB/sec)",
        msgs_per_sec, bytes_per_sec / 1_000_000.0);
    println!();
}
