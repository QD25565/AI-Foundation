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
use teamengram::wake::signal_sequencer;

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

/// Singleton guard — prevents multiple daemon instances.
/// Dropping this releases the lock (process exit also releases automatically).
struct SingletonGuard {
    #[cfg(windows)]
    _handle: isize, // HANDLE to Named Mutex
    #[cfg(not(windows))]
    _file: std::fs::File, // File with flock held
}

#[cfg(windows)]
fn acquire_singleton(_data_dir: &PathBuf) -> SingletonGuard {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};

    // Named Mutex is a kernel object — survives file deletion, not tied to filesystem.
    // Name is global to the Windows session (Local\ namespace).
    let name: Vec<u16> = "Local\\TeamEngram_V2_Daemon_Singleton\0"
        .encode_utf16()
        .collect();

    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle == 0 {
        eprintln!("CRITICAL: Failed to create singleton mutex");
        std::process::exit(1);
    }

    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_ALREADY_EXISTS {
        eprintln!("|ALREADY RUNNING|");
        eprintln!("Hint: Another v2-daemon instance holds the singleton mutex.");
        std::process::exit(0);
    }

    eprintln!("[SINGLETON] Acquired Named Mutex (kernel-level)");
    SingletonGuard { _handle: handle }
}

#[cfg(not(windows))]
fn acquire_singleton(data_dir: &PathBuf) -> SingletonGuard {
    let lock_path = data_dir.join("v2-daemon.lock");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(&lock_path)
        .unwrap_or_else(|e| {
            eprintln!("CRITICAL: Failed to open lock file: {}", e);
            std::process::exit(1);
        });

    // flock is tied to the file descriptor, not the path.
    // Deleting the lock file does NOT release the lock — the kernel tracks it by inode.
    // Lock is automatically released when the process exits (even on crash/kill).
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        eprintln!("|ALREADY RUNNING|");
        eprintln!("Hint: Another v2-daemon instance holds the file lock.");
        std::process::exit(0);
    }

    eprintln!("[SINGLETON] Acquired flock (kernel-level)");
    SingletonGuard { _file: file }
}

fn run_daemon(data_dir: &PathBuf) {
    // Singleton enforcement: kernel-level mutual exclusion
    // Windows: Named Mutex (survives file deletion, kernel-managed)
    // Linux: flock() on lock file (released automatically on process exit)
    let _singleton_guard = acquire_singleton(data_dir);

    // Write PID to lock file (informational only — not used for locking)
    use std::io::Write;
    let lock_path = data_dir.join("v2-daemon.lock");
    if let Ok(mut f) = std::fs::File::create(&lock_path) {
        let _ = writeln!(f, "{}", std::process::id());
    }

    eprintln!("|V2 DAEMON STARTING|");
    eprintln!("PID:{}", std::process::id());
    eprintln!("DataDir:{}", data_dir.display());

    // Report event log state
    let log_path = data_dir.join("shared").join("events").join("master.eventlog");
    if log_path.exists() {
        if let Ok(meta) = std::fs::metadata(&log_path) {
            eprintln!("EventLog:{}KB", meta.len() / 1024);
        }
    }

    // Report outbox state
    let outbox_dir = data_dir.join("shared").join("outbox");
    let mut outbox_count = 0;
    if let Ok(entries) = std::fs::read_dir(&outbox_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map_or(false, |e| e == "outbox") {
                outbox_count += 1;
                if let Some(ai_id) = entry.path().file_stem() {
                    eprintln!("FoundOutbox:{}", ai_id.to_string_lossy());
                }
            }
        }
    }
    eprintln!("Outboxes:{}", outbox_count);

    // Set up Ctrl+C handler.
    // MUST signal the sequencer wake semaphore after setting stop flag —
    // the sequencer blocks on sem_wait() with no timeout, so without this
    // signal it would never wake up to check stop_signal.
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_signal.clone();
    ctrlc::set_handler(move || {
        eprintln!("\n|SHUTDOWN REQUESTED|");
        stop_clone.store(true, Ordering::SeqCst);
        signal_sequencer(); // Wake the sequencer so it sees stop_signal
    }).expect("Error setting Ctrl-C handler");

    // Configure sequencer
    let config = SequencerConfig {
        base_dir: Some(data_dir.clone()),
        enable_wake: true,
        ..Default::default()
    };

    // Create and run sequencer
    let mut sequencer = match Sequencer::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("CRITICAL: Failed to create sequencer: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("|RUNNING|");
    eprintln!("Sequence:{}", sequencer.current_sequence());

    // Run the sequencer (blocks until stop signal)
    match sequencer.run(stop_signal) {
        Ok(()) => {
            let stats = sequencer.stats();
            eprintln!("|SHUTDOWN COMPLETE|");
            eprintln!("EventsProcessed:{}", stats.events_processed.load(Ordering::Relaxed));
            eprintln!("BatchesProcessed:{}", stats.batches_processed.load(Ordering::Relaxed));
        }
        Err(e) => {
            eprintln!("CRITICAL: Sequencer failed: {}", e);
            std::process::exit(1);
        }
    }
}
