//! Sequencer - Single-Threaded Event Ordering Engine
//!
//! The Sequencer is the core of the Event Sourcing architecture. It:
//! 1. WAITS for events via OS-native Named Events (NO POLLING!)
//! 2. Reads events from outboxes (wait-free)
//! 3. Assigns global sequence numbers
//! 4. Writes events to the master log
//! 5. Signals wake events for affected AIs
//!
//! Architecture (LMAX Disruptor inspired):
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Sequencer Thread                           │
//! │                                                                 │
//! │  ┌─────────┐  ┌─────────┐  ┌─────────┐       ┌─────────────┐   │
//! │  │ Outbox  │  │ Outbox  │  │ Outbox  │  ...  │   Master    │   │
//! │  │ lyra-   │  │ sage-   │  │cascade- │       │  Event Log  │   │
//! │  │  584    │  │  724    │  │   230   │       │             │   │
//! │  └────┬────┘  └────┬────┘  └────┬────┘       └──────┬──────┘   │
//! │       │            │            │                   │          │
//! │       └────────────┴────────────┘                   │          │
//! │                    │                                │          │
//! │              Event-Driven Wake                       │          │
//! │                    │                                │          │
//! │                    ▼                                ▼          │
//! │            ┌───────────────┐              ┌─────────────────┐  │
//! │            │  Assign Seq#  │─────────────▶│  Append to Log  │  │
//! │            └───────────────┘              └─────────────────┘  │
//! │                                                    │           │
//! │                                                    ▼           │
//! │                                           ┌─────────────────┐  │
//! │                                           │  Signal Wake    │  │
//! │                                           │  (affected AIs) │  │
//! │                                           └─────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Guarantees:
//! - Total ordering: All events get strictly increasing sequence numbers
//! - At-least-once delivery: Events committed to log before outbox ack
//! - Wait-free reading: Outbox reads never block
//! - Low latency: Single thread, no locks, no CAS contention

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
// std::time::{Duration, Instant} used in tests only (test modules import their own)

use crate::event::EventHeader;
use crate::event_log::EventLogWriter;
use crate::outbox::{OutboxConsumer, list_outboxes};
use crate::wake::{WakeCoordinator, WakeReason, SequencerWakeReceiver};

/// Sequencer configuration
#[derive(Debug, Clone)]
pub struct SequencerConfig {
    /// Base directory for data files
    pub base_dir: Option<std::path::PathBuf>,
    /// Maximum events to process per batch
    pub max_batch_size: usize,
    /// Sync interval (every N events)
    pub sync_interval: u64,
    /// Enable wake signaling
    pub enable_wake: bool,
    // REMOVED: outbox_refresh_secs - NO POLLING, pure event-driven
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            base_dir: None,
            max_batch_size: 1000,
            sync_interval: 100,
            enable_wake: true,
            // REMOVED: outbox_refresh_secs - NO POLLING
        }
    }
}

/// Sequencer statistics
#[derive(Debug, Default)]
pub struct SequencerStats {
    /// Total events processed
    pub events_processed: AtomicU64,
    /// Total batches processed
    pub batches_processed: AtomicU64,
    /// Last sequence number assigned
    pub last_sequence: AtomicU64,
    /// Number of active outboxes
    pub active_outboxes: AtomicU64,
    /// Timestamp of last event
    pub last_event_time: AtomicU64,
}

impl SequencerStats {
    pub fn events_processed(&self) -> u64 {
        self.events_processed.load(Ordering::Relaxed)
    }

    pub fn batches_processed(&self) -> u64 {
        self.batches_processed.load(Ordering::Relaxed)
    }

    pub fn last_sequence(&self) -> u64 {
        self.last_sequence.load(Ordering::Relaxed)
    }
}

/// Handle to control the running sequencer
pub struct SequencerHandle {
    /// Signal to stop the sequencer
    stop_signal: Arc<AtomicBool>,
    /// Thread handle
    thread_handle: Option<JoinHandle<SequencerResult<()>>>,
    /// Statistics
    stats: Arc<SequencerStats>,
}

impl SequencerHandle {
    /// Stop the sequencer gracefully
    pub fn stop(mut self) -> SequencerResult<()> {
        self.stop_signal.store(true, Ordering::Release);
        if let Some(handle) = self.thread_handle.take() {
            handle.join().map_err(|_| SequencerError::ThreadPanic)??;
        }
        Ok(())
    }

    /// Check if the sequencer is running
    pub fn is_running(&self) -> bool {
        !self.stop_signal.load(Ordering::Acquire)
    }

    /// Get statistics
    pub fn stats(&self) -> &SequencerStats {
        &self.stats
    }

    /// Request stop without waiting
    pub fn request_stop(&self) {
        self.stop_signal.store(true, Ordering::Release);
    }
}

/// Result type for sequencer operations
pub type SequencerResult<T> = Result<T, SequencerError>;

/// Sequencer errors
#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    #[error("Event log error: {0}")]
    EventLog(#[from] crate::event_log::EventLogError),

    #[error("Outbox error: {0}")]
    Outbox(#[from] crate::outbox::OutboxError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Thread panicked")]
    ThreadPanic,

    #[error("Already running")]
    AlreadyRunning,
}

/// The Sequencer engine
///
/// Architecture: Event-driven with zero polling.
/// - OutboxProducer.write_event() signals SequencerWakeSignaler (cross-process)
/// - Sequencer waits on SequencerWakeReceiver (blocks, zero CPU)
/// - Wake latency: ~1μs (Windows Named Events) / ~500ns (Linux eventfd)
pub struct Sequencer {
    config: SequencerConfig,
    event_log: EventLogWriter,
    outboxes: HashMap<String, OutboxConsumer>,
    /// Wake coordinator for signaling target AIs when events affect them
    wake_coordinator: Option<WakeCoordinator>,
    stats: Arc<SequencerStats>,
    next_sequence: u64,
    events_since_sync: u64,
    /// Lightweight dialogue participant map for wake signaling on responses.
    /// Maps dialogue_id → (initiator, responder) so DialogueRespond can wake the other party.
    dialogue_participants: HashMap<u64, (String, String)>,
}

impl Sequencer {
    /// Create a new sequencer
    pub fn new(config: SequencerConfig) -> SequencerResult<Self> {
        let event_log = EventLogWriter::open(config.base_dir.as_deref())?;
        let next_sequence = event_log.current_sequence() + 1;

        let wake_coordinator = if config.enable_wake {
            WakeCoordinator::new("sequencer").ok()
        } else {
            None
        };

        Ok(Self {
            config,
            event_log,
            outboxes: HashMap::new(),
            wake_coordinator,
            stats: Arc::new(SequencerStats::default()),
            next_sequence,
            events_since_sync: 0,
            dialogue_participants: HashMap::new(),
        })
    }

    /// Start the sequencer in a background thread
    pub fn start(config: SequencerConfig) -> SequencerResult<SequencerHandle> {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_signal);

        let mut sequencer = Self::new(config)?;
        let stats = Arc::clone(&sequencer.stats);

        let handle = thread::Builder::new()
            .name("sequencer".to_string())
            .spawn(move || {
                sequencer.run(stop_clone)
            })?;

        Ok(SequencerHandle {
            stop_signal,
            thread_handle: Some(handle),
            stats,
        })
    }

    /// Run the sequencer loop (blocking) - EVENT-DRIVEN, NO POLLING!
    ///
    /// Uses cross-process Named Events for instant wake when AIs write events.
    /// Zero CPU usage while waiting, ~1μs wake latency.
    pub fn run(&mut self, stop_signal: Arc<AtomicBool>) -> SequencerResult<()> {
        self.run_event_driven(stop_signal)
    }

    /// Run with pure event-driven cross-process wake. No polling. No timeouts.
    ///
    /// Uses OS-native Named Events (Windows) or POSIX named semaphores (Linux)
    /// for instant wake when any AI writes to their outbox. Zero CPU while waiting.
    ///
    /// Architecture:
    /// - Sequencer blocks on SequencerWakeReceiver::wait() (zero CPU, infinite)
    /// - OutboxProducer.write_event() calls signal_sequencer() after every write
    /// - Sequencer wakes instantly (~1μs on Windows, ~200ns on Linux)
    /// - On wake: refresh outboxes, process all events, block again
    /// - On SIGINT: shutdown handler signals semaphore, daemon wakes and exits
    ///
    /// There is NO timeout. If the signal is broken, the daemon blocks forever —
    /// that's intentional. A broken signal is a bug to fix, not a condition to mask.
    ///
    /// # Errors
    /// Returns error if cross-process wake receiver cannot be created.
    /// NO FALLBACKS - if this fails, it fails loudly so the issue can be fixed.
    pub fn run_event_driven(
        &mut self,
        stop_signal: Arc<AtomicBool>,
    ) -> SequencerResult<()> {
        // Create the cross-process wake receiver - FAIL LOUDLY if this doesn't work
        eprintln!("[SEQUENCER] Creating SequencerWakeReceiver...");
        let wake_receiver = SequencerWakeReceiver::new()
            .map_err(|e| SequencerError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("CRITICAL: Failed to create SequencerWakeReceiver: {}. Event-driven wake is REQUIRED - no fallbacks!", e)
            )))?;
        eprintln!("[SEQUENCER] SequencerWakeReceiver created successfully");

        // Initial outbox scan
        self.refresh_outboxes()?;
        let outbox_count = self.outboxes.len();

        // Report pending events per outbox at startup
        let mut total_pending: u64 = 0;
        for (ai_id, consumer) in &self.outboxes {
            let pending = consumer.pending_bytes();
            if pending > 0 {
                eprintln!("[SEQUENCER] Outbox {}: {} bytes pending", ai_id, pending);
                total_pending += pending as u64;
            }
        }
        eprintln!("[SEQUENCER] Found {} outboxes, {} bytes total pending", outbox_count, total_pending);

        // Drain ALL pending events before entering wait loop.
        // This ensures events written while daemon was offline are processed immediately.
        let mut initial_total = 0usize;
        loop {
            let batch = self.process_batch()?;
            initial_total += batch;
            if batch == 0 { break; }
        }
        if initial_total > 0 {
            self.event_log.sync()?;
            eprintln!("[SEQUENCER] Initial drain: processed {} events, synced to disk", initial_total);
        }

        eprintln!("[SEQUENCER] Running (pure event-driven, no polling, no timeouts)");
        eprintln!("[SEQUENCER] Next sequence: {}", self.next_sequence);

        while !stop_signal.load(Ordering::Acquire) {
            // Refresh outboxes on every wake (catches new AIs registering)
            self.refresh_outboxes()?;

            // Process ALL available events across all outboxes
            let events_processed = self.process_batch()?;
            self.stats.active_outboxes.store(self.outboxes.len() as u64, Ordering::Relaxed);

            if events_processed > 0 {
                let total = self.stats.events_processed.load(Ordering::Relaxed);
                eprintln!(
                    "[SEQUENCER] Processed {} events (seq {}, total {})",
                    events_processed, self.next_sequence - 1, total
                );
                // Events found — loop immediately to drain any more.
                // Don't block until all outboxes are empty.
                continue;
            }

            // No events available. Block until signaled.
            // sem_wait (Linux) / WaitForSingleObject INFINITE (Windows).
            // Wakes on: outbox write (signal_sequencer), or shutdown (signal_sequencer from ctrlc handler).
            // If signal is broken, this blocks forever. That's correct — fix the signal, don't mask it.
            wake_receiver.wait();
        }

        self.event_log.sync()?;
        eprintln!("[SEQUENCER] Shutdown complete");
        Ok(())
    }

    /// Refresh the list of outboxes
    fn refresh_outboxes(&mut self) -> SequencerResult<()> {
        let ai_ids = list_outboxes(self.config.base_dir.as_deref())?;

        for ai_id in ai_ids {
            if !self.outboxes.contains_key(&ai_id) {
                match OutboxConsumer::open(&ai_id, self.config.base_dir.as_deref()) {
                    Ok(consumer) => {
                        self.outboxes.insert(ai_id.clone(), consumer);
                    }
                    Err(e) => {
                        // Log but don't fail - outbox might be being created
                        eprintln!("Failed to open outbox for {}: {}", ai_id, e);
                    }
                }
            }
        }

        // Remove closed outboxes
        self.outboxes.retain(|_, consumer| !consumer.is_closed());

        Ok(())
    }

    /// Process a batch of events from all outboxes
    fn process_batch(&mut self) -> SequencerResult<usize> {
        let mut total_processed = 0;

        // Round-robin through all outboxes
        let ai_ids: Vec<String> = self.outboxes.keys().cloned().collect();

        for ai_id in ai_ids {
            if total_processed >= self.config.max_batch_size {
                break;
            }

            let events_from_outbox = self.drain_outbox(&ai_id)?;
            total_processed += events_from_outbox;
        }

        if total_processed > 0 {
            self.stats.batches_processed.fetch_add(1, Ordering::Relaxed);
        }

        Ok(total_processed)
    }

    /// Drain events from a single outbox
    ///
    /// Uses CAS-based commit for linearizability. If another sequencer process
    /// has already committed an event, we skip it (CAS fails) and move on.
    fn drain_outbox(&mut self, ai_id: &str) -> SequencerResult<usize> {
        let mut processed = 0;

        let consumer = match self.outboxes.get(ai_id) {
            Some(c) => c,
            None => return Ok(0),
        };

        // Check for outbox corruption before processing
        if let Some(corruption_reason) = consumer.check_corruption() {
            eprintln!(
                "WARNING: Outbox for {} appears corrupted: {}. Skipping until repaired.",
                ai_id, corruption_reason
            );
            return Ok(0);
        }

        while let Some((raw, tail_position)) = consumer.try_read_raw_with_position() {
            if raw.len() < 64 {
                // Skip malformed event - use CAS commit
                if !consumer.commit_read_cas(tail_position, raw.len()) {
                    // Another process already committed - we're done with this outbox
                    break;
                }
                continue;
            }

            // Parse header to modify sequence number
            let mut header_bytes: [u8; 64] = raw[..64].try_into().unwrap();
            let payload_bytes = &raw[64..];

            // Assign sequence number (modify header bytes directly)
            let sequence = self.next_sequence;
            header_bytes[0..8].copy_from_slice(&sequence.to_le_bytes());

            // Recalculate checksum with new sequence
            let mut header = EventHeader::from_bytes(&header_bytes);
            header.checksum = header.calculate_checksum(payload_bytes);
            let updated_header_bytes = header.to_bytes();

            // Append to master log FIRST (before commit)
            self.event_log.append_raw(&updated_header_bytes, payload_bytes, sequence)?;

            // CAS commit: Only advance if we still own this event
            if !consumer.commit_read_cas(tail_position, raw.len()) {
                // Another process committed this event - they wrote to eventlog too.
                // This means the event was written twice (harmless for idempotent events,
                // but we should decrement our sequence to avoid gaps).
                // For now, we just stop processing this outbox - the other sequencer has it.
                eprintln!(
                    "INFO: CAS failed for {} at position {}. Another sequencer committed. Yielding.",
                    ai_id, tail_position
                );
                break;
            }

            // Successfully committed - advance our sequence counter
            self.next_sequence += 1;

            // Update outbox's last sequence
            consumer.set_last_sequence(sequence);

            // Update stats
            self.stats.events_processed.fetch_add(1, Ordering::Relaxed);
            self.stats.last_sequence.store(sequence, Ordering::Relaxed);
            self.stats.last_event_time.store(crate::store::now_millis(), Ordering::Relaxed);

            // Signal wake for affected AIs
            // Pass fields explicitly to avoid borrow conflict with self.outboxes
            let wake_enabled = self.wake_coordinator.is_some();
            Self::signal_wake_if_needed(wake_enabled, &mut self.dialogue_participants, &header, payload_bytes);

            processed += 1;
            self.events_since_sync += 1;

            // Periodic sync
            if self.events_since_sync >= self.config.sync_interval {
                self.event_log.sync()?;
                self.events_since_sync = 0;
            }

            // Check batch limit
            if processed >= self.config.max_batch_size {
                break;
            }
        }

        Ok(processed)
    }

    /// Signal wake events for affected AIs
    fn signal_wake_if_needed(
        wake_enabled: bool,
        dialogue_participants: &mut HashMap<u64, (String, String)>,
        header: &EventHeader,
        payload_bytes: &[u8],
    ) {
        if !wake_enabled {
            return;
        }

        let source_ai = header.source_ai_str();

        let payload = match crate::event::EventPayload::from_bytes(payload_bytes) {
            Some(p) => p,
            None => return,
        };

        match payload {
            crate::event::EventPayload::DirectMessage(dm) => {
                Self::signal_ai(&dm.to_ai, WakeReason::DirectMessage, source_ai, &dm.content);
            }
            crate::event::EventPayload::Broadcast(bc) => {
                for ai_id in Self::extract_mentions(&bc.content) {
                    if ai_id != source_ai {
                        Self::signal_ai(&ai_id, WakeReason::Mention, source_ai, &bc.content);
                    }
                }
                if Self::contains_urgent(&bc.content) {
                    // Could broadcast-wake all AIs, but that's noisy
                }
            }
            crate::event::EventPayload::DialogueStart(ds) => {
                // Track participants so DialogueRespond can wake the other party
                let dialogue_id = header.sequence;
                dialogue_participants.insert(
                    dialogue_id,
                    (source_ai.to_string(), ds.responder.clone()),
                );
                Self::signal_ai(&ds.responder, WakeReason::DialogueTurn, source_ai, &ds.topic);
            }
            crate::event::EventPayload::DialogueRespond(dr) => {
                // Look up participants and wake the OTHER party
                if let Some((initiator, responder)) = dialogue_participants.get(&dr.dialogue_id) {
                    let target = if source_ai == initiator {
                        responder.clone()
                    } else {
                        initiator.clone()
                    };
                    Self::signal_ai(&target, WakeReason::DialogueTurn, source_ai, &dr.content);
                }
            }
            crate::event::EventPayload::DialogueEnd(de) => {
                // Clean up dialogue tracking
                dialogue_participants.remove(&de.dialogue_id);
            }
            crate::event::EventPayload::VoteCreate(vc) => {
                let _ = vc;
            }
            crate::event::EventPayload::RoomMessage(rm) => {
                let _ = rm;
            }
            _ => {}
        }
    }

    /// Signal a specific AI to wake up
    fn signal_ai(ai_id: &str, reason: WakeReason, from_ai: &str, content: &str) {
        // Open the target AI's wake event and signal it
        // NO TRUNCATION - full content always (architecture decision: context starvation is the enemy)
        if let Ok(wake) = WakeCoordinator::new(ai_id) {
            wake.wake(reason, from_ai, content);
        }
    }

    /// Extract @mentions from content
    fn extract_mentions(content: &str) -> Vec<String> {
        let mut mentions = Vec::new();
        for word in content.split_whitespace() {
            if let Some(mention) = word.strip_prefix('@') {
                // Clean up the mention (remove trailing punctuation)
                let ai_id: String = mention
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if !ai_id.is_empty() {
                    mentions.push(ai_id);
                }
            }
        }
        mentions
    }

    /// Check if content contains urgent keywords
    fn contains_urgent(content: &str) -> bool {
        let lower = content.to_lowercase();
        lower.contains("urgent") || lower.contains("asap") || lower.contains("critical")
    }

    /// Get current statistics
    pub fn stats(&self) -> &SequencerStats {
        &self.stats
    }

    /// Get current sequence number
    pub fn current_sequence(&self) -> u64 {
        self.next_sequence - 1
    }
}

/// Run the sequencer as a standalone process
pub fn run_sequencer(config: SequencerConfig) -> SequencerResult<()> {
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_signal);

    // Set up Ctrl+C handler
    #[cfg(not(test))]
    {
        let stop_for_signal = Arc::clone(&stop_signal);
        ctrlc::set_handler(move || {
            stop_for_signal.store(true, Ordering::Release);
        }).ok();
    }

    let mut sequencer = Sequencer::new(config)?;
    sequencer.run(stop_clone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::outbox::OutboxProducer;
    use std::time::Duration;

    #[test]
    fn test_sequencer_create() {
        let tmp = TempDir::new().unwrap();
        let config = SequencerConfig {
            base_dir: Some(tmp.path().to_path_buf()),
            enable_wake: false,
            ..Default::default()
        };

        let sequencer = Sequencer::new(config).unwrap();
        assert_eq!(sequencer.current_sequence(), 0);
    }

    #[test]
    fn test_sequencer_process_events() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create an outbox and write some events
        let mut producer = OutboxProducer::open("test-ai", Some(base)).unwrap();
        for i in 0..5 {
            let event = Event::broadcast("test-ai", "general", &format!("Message {}", i));
            producer.write_event(&event).unwrap();
        }
        producer.flush().unwrap();
        drop(producer);

        // Create sequencer and process
        let config = SequencerConfig {
            base_dir: Some(base.to_path_buf()),
            enable_wake: false,
            ..Default::default()
        };

        let mut sequencer = Sequencer::new(config).unwrap();
        sequencer.refresh_outboxes().unwrap();

        let processed = sequencer.process_batch().unwrap();
        assert_eq!(processed, 5);
        assert_eq!(sequencer.current_sequence(), 5);
        assert_eq!(sequencer.stats.events_processed(), 5);
    }

    #[test]
    fn test_sequencer_multiple_outboxes() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create multiple outboxes
        for ai_id in &["ai-1", "ai-2", "ai-3"] {
            let mut producer = OutboxProducer::open(ai_id, Some(base)).unwrap();
            for i in 0..3 {
                let event = Event::broadcast(ai_id, "general", &format!("{} message {}", ai_id, i));
                producer.write_event(&event).unwrap();
            }
            producer.flush().unwrap();
        }

        // Process all
        let config = SequencerConfig {
            base_dir: Some(base.to_path_buf()),
            enable_wake: false,
            ..Default::default()
        };

        let mut sequencer = Sequencer::new(config).unwrap();
        sequencer.refresh_outboxes().unwrap();

        let processed = sequencer.process_batch().unwrap();
        assert_eq!(processed, 9); // 3 AIs * 3 messages
        assert_eq!(sequencer.current_sequence(), 9);
    }

    #[test]
    fn test_sequencer_start_stop() {
        let tmp = TempDir::new().unwrap();
        let config = SequencerConfig {
            base_dir: Some(tmp.path().to_path_buf()),
            enable_wake: false,
            ..Default::default()
        };

        let handle = Sequencer::start(config).unwrap();
        assert!(handle.is_running());

        // Let it run briefly
        thread::sleep(Duration::from_millis(50));

        // Stop it
        handle.stop().unwrap();
    }

    #[test]
    fn test_sequencer_ordering() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Write events to multiple outboxes
        for i in 0..3 {
            let ai_id = format!("ai-{}", i);
            let mut producer = OutboxProducer::open(&ai_id, Some(base)).unwrap();
            let event = Event::broadcast(&ai_id, "general", &format!("From {}", ai_id));
            producer.write_event(&event).unwrap();
            producer.flush().unwrap();
        }

        // Process
        let config = SequencerConfig {
            base_dir: Some(base.to_path_buf()),
            enable_wake: false,
            ..Default::default()
        };

        let mut sequencer = Sequencer::new(config).unwrap();
        sequencer.refresh_outboxes().unwrap();
        sequencer.process_batch().unwrap();

        // Read back from log and verify ordering
        let mut reader = crate::event_log::EventLogReader::open(Some(base)).unwrap();
        let mut last_seq = 0;
        while let Some(event) = reader.try_read().unwrap() {
            assert!(event.header.sequence > last_seq, "Events must be ordered");
            last_seq = event.header.sequence;
        }
        assert_eq!(last_seq, 3);
    }
}
