//! TeamEngram V2 Daemon
//!
//! Runs the Sequencer continuously to:
//\! - Read all AI outboxes (event-driven)
//! - Write events to master event log
//! - Signal wake events for relevant AIs
//!
//! Usage:
//!   v2-daemon                    # Run daemon
//!   v2-daemon --register lyra-584  # Register an AI's outbox

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::{Parser, Subcommand};

use teamengram::sequencer::{Sequencer, SequencerConfig};
use teamengram::outbox::OutboxProducer;

#[derive(Parser)]
#[command(name = "v2-daemon")]
#[command(about = "TeamEngram V2 Event Sequencer Daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Data directory
    #[arg(long, short = 'd')]
    data_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register an AI's outbox with the sequencer
    Register {
        /// AI ID to register
        ai_id: String,
    },

    /// Show daemon status
    Status,
}

fn main() {
    let cli = Cli::parse();

    let data_dir = cli.data_dir.unwrap_or_else(|| {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ai-foundation")
            .join("v2")
    });

    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    match cli.command {
        Some(Commands::Register { ai_id }) => {
            register_ai(&data_dir, &ai_id);
        }
        Some(Commands::Status) => {
            show_status(&data_dir);
        }
        None => {
            run_daemon(&data_dir);
        }
    }
}

fn register_ai(data_dir: &PathBuf, ai_id: &str) {
    // Create outbox for the AI (this registers it)
    match OutboxProducer::open(ai_id, Some(data_dir)) {
        Ok(_) => {
            println!("|REGISTERED|");
            println!("AI:{}", ai_id);
            println!("Outbox:{}", data_dir.join("shared").join("outbox").join(format!("{}.outbox", ai_id)).display());
        }
        Err(e) => {
            eprintln!("Error: Failed to register AI: {}", e);
            std::process::exit(1);
        }
    }
}

fn show_status(data_dir: &PathBuf) {
    let outbox_dir = data_dir.join("shared").join("outbox");

    println!("|V2 STATUS|");
    println!("DataDir:{}", data_dir.display());

    // Count registered AIs
    let mut ai_count = 0;
    if let Ok(entries) = std::fs::read_dir(&outbox_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map_or(false, |e| e == "outbox") {
                ai_count += 1;
                if let Some(name) = entry.path().file_stem() {
                    println!("  AI:{}", name.to_string_lossy());
                }
            }
        }
    }
    println!("RegisteredAIs:{}", ai_count);

    // Check event log
    let log_path = data_dir.join("shared").join("events").join("master.eventlog");
    if log_path.exists() {
        if let Ok(meta) = std::fs::metadata(&log_path) {
            println!("EventLog:{}KB", meta.len() / 1024);
        }
    } else {
        println!("EventLog:None");
    }
}

fn run_daemon(data_dir: &PathBuf) {
    // Single-instance lock file
    let lock_path = data_dir.join("v2-daemon.lock");
    let lock_file = match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: Failed to create lock file: {}", e);
            std::process::exit(1);
        }
    };

    // Try to acquire exclusive lock - if fails, another daemon is running
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::IO::OVERLAPPED;

        let handle = lock_file.as_raw_handle() as HANDLE;
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };

        let result = unsafe {
            LockFileEx(
                handle,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };

        if result == 0 {
            // Lock failed - another daemon is running
            eprintln!("|ALREADY RUNNING|");
            eprintln!("Hint: Another v2-daemon instance is running. Only one needed.");
            std::process::exit(0); // Exit cleanly, not an error
        }
    }

    // Write PID to lock file
    use std::io::Write;
    let mut lock_file = lock_file;
    let _ = writeln!(lock_file, "{}", std::process::id());

    println!("|V2 DAEMON STARTING|");
    println!("DataDir:{}", data_dir.display());

    // Set up Ctrl+C handler
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_signal.clone();
    ctrlc::set_handler(move || {
        println!("\n|SHUTDOWN REQUESTED|");
        stop_clone.store(true, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");

    // Configure sequencer
    let config = SequencerConfig {
        base_dir: Some(data_dir.clone()),
        enable_wake: true,
        ..Default::default()
    };

    // List existing outboxes
    let outbox_dir = data_dir.join("shared").join("outbox");
    if let Ok(entries) = std::fs::read_dir(&outbox_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map_or(false, |e| e == "outbox") {
                if let Some(ai_id) = entry.path().file_stem() {
                    println!("FoundOutbox:{}", ai_id.to_string_lossy());
                }
            }
        }
    }

    // Create and run sequencer
    let mut sequencer = match Sequencer::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: Failed to create sequencer: {}", e);
            std::process::exit(1);
        }
    };

    println!("|RUNNING|");

    // Run the sequencer (blocks until stop signal)
    match sequencer.run(stop_signal) {
        Ok(()) => {
            let stats = sequencer.stats();
            println!("|SHUTDOWN COMPLETE|");
            println!("EventsProcessed:{}", stats.events_processed.load(Ordering::Relaxed));
            println!("BatchesProcessed:{}", stats.batches_processed.load(Ordering::Relaxed));
        }
        Err(e) => {
            eprintln!("Error: Sequencer failed: {}", e);
            std::process::exit(1);
        }
    }
}
