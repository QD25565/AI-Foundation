//! Cross-Platform Wake Events
//!
//! Zero-polling, OS-native event primitives for instant AI wake-up.
//! Each platform uses its optimal primitive:
//!
//! - Windows: Named Events (CreateEventW / WaitForSingleObject)
//! - Linux/macOS: POSIX named semaphores
//!
//! Latency: ~500ns-1us wake time, zero CPU usage while waiting.
//!
//! Cross-process wake is a pure signal. Waiters query the event log/view after
//! waking to learn what changed; the in-process reason field is only reliable
//! when the same process both signals and waits.

use std::time::Duration;

/// Reason for wake event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WakeReason {
    /// No wake (timeout expired)
    None = 0,
    /// Direct message received
    DirectMessage = 1,
    /// Mentioned in broadcast
    Mention = 2,
    /// Urgent keyword detected
    Urgent = 3,
    /// Task assigned
    TaskAssigned = 4,
    /// Manual wake request
    Manual = 5,
    /// Broadcast received (general)
    Broadcast = 6,
    /// Dialogue turn
    DialogueTurn = 7,
    /// Vote requires attention
    VoteRequest = 8,
    /// File claim released by another AI
    FileReleased = 9,
}

impl From<u8> for WakeReason {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::DirectMessage,
            2 => Self::Mention,
            3 => Self::Urgent,
            4 => Self::TaskAssigned,
            5 => Self::Manual,
            6 => Self::Broadcast,
            7 => Self::DialogueTurn,
            8 => Self::VoteRequest,
            9 => Self::FileReleased,
            _ => Self::None,
        }
    }
}

/// Result of waiting for a wake event
#[derive(Debug, Clone)]
pub struct WakeResult {
    pub reason: WakeReason,
    pub from_ai: Option<String>,
    pub content_preview: Option<String>,
}

impl WakeResult {
    pub fn timeout() -> Self {
        Self {
            reason: WakeReason::None,
            from_ai: None,
            content_preview: None,
        }
    }

    pub fn new(reason: WakeReason, from_ai: Option<String>, content: Option<String>) -> Self {
        Self {
            reason,
            from_ai,
            content_preview: content,
        }
    }
}

/// Cross-platform wake event trait
pub trait WakeEvent: Send + Sync {
    /// Block until signaled. Returns wake reason.
    fn wait(&self) -> WakeResult;

    /// Block until signaled or timeout. Returns None on timeout.
    fn wait_timeout(&self, timeout: Duration) -> Option<WakeResult>;

    /// Signal the event to wake waiting thread.
    fn signal(&self, reason: WakeReason, from_ai: &str, content: &str);

    /// Check if event is signaled without blocking.
    fn try_recv(&self) -> Option<WakeResult>;
}

// ============================================================================
// WINDOWS IMPLEMENTATION - Named Events (pure signal, no file metadata)
// ============================================================================

#[cfg(target_os = "windows")]
pub mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::sync::atomic::{AtomicU8, Ordering};

    // Windows API constants
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;
    const INFINITE: u32 = 0xFFFFFFFF;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x00100000;

    // NOTE: File-based metadata was REMOVED. It caused stale wake bugs where the
    // AI would repeatedly wake on old data because the file wasn't cleared properly.
    // The Named Event is a pure signal - after waking, the CLI queries the
    // VIEW (the source of truth) to find out what actually arrived.
    //
    // The AtomicU8 reason field stores wake reason in-process only. For same-process
    // signaling (standby command, tests) the reason propagates correctly. For
    // cross-process named events, the reason is process-local and is not visible
    // to the waiter — it returns WakeReason::None, and the AI queries the view.

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lpEventAttributes: *mut std::ffi::c_void,
            bManualReset: i32,
            bInitialState: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;

        fn OpenEventW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;

        fn SetEvent(hEvent: *mut std::ffi::c_void) -> i32;
        fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }

    /// Windows Named Event implementation - pure signal with in-process reason storage
    pub struct WindowsWakeEvent {
        handle: *mut std::ffi::c_void,
        /// In-process wake reason. Propagates correctly for same-process signaling.
        /// Cross-process callers get WakeReason::None and query the view instead.
        reason: AtomicU8,
    }

    // SAFETY: Windows handles are thread-safe when used correctly
    unsafe impl Send for WindowsWakeEvent {}
    unsafe impl Sync for WindowsWakeEvent {}

    impl WindowsWakeEvent {
        /// Create or open a named wake event for an AI
        pub fn open(ai_id: &str) -> std::io::Result<Self> {
            // Use Local\ prefix (works without admin) instead of Global\ (requires admin)
            let name = format!("Local\\TeamEngram_Wake_{}", ai_id);
            eprintln!("[WAKE] Opening event for ai_id='{}', name='{}'", ai_id, name);
            let wide_name: Vec<u16> = OsStr::new(&name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            // Try to open existing event first
            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE | SYNCHRONIZE,
                    0, // Do not inherit
                    wide_name.as_ptr(),
                )
            };

            let handle = if handle.is_null() {
                let err = std::io::Error::last_os_error();
                eprintln!("[WAKE] OpenEventW FAILED for '{}': {} - creating new event", name, err);
                // Create new event (auto-reset, initially non-signaled)
                let h = unsafe {
                    CreateEventW(
                        ptr::null_mut(),
                        0, // Auto-reset
                        0, // Initially non-signaled
                        wide_name.as_ptr(),
                    )
                };

                if h.is_null() {
                    let create_err = std::io::Error::last_os_error();
                    eprintln!("[WAKE] CreateEventW FAILED for '{}': {}", name, create_err);
                    return Err(create_err);
                }
                eprintln!("[WAKE] CreateEventW SUCCESS for '{}', handle={:?}", name, h);
                h
            } else {
                eprintln!("[WAKE] OpenEventW SUCCESS for '{}', handle={:?}", name, handle);
                handle
            };

            Ok(Self { handle, reason: AtomicU8::new(0) })
        }

        /// Create anonymous event (same-process only)
        pub fn new() -> std::io::Result<Self> {
            let handle = unsafe {
                CreateEventW(
                    ptr::null_mut(),
                    0, // Auto-reset
                    0, // Initially non-signaled
                    ptr::null(),
                )
            };

            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }

            Ok(Self { handle, reason: AtomicU8::new(0) })
        }
    }

    impl Drop for WindowsWakeEvent {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    impl WakeEvent for WindowsWakeEvent {
        fn wait(&self) -> WakeResult {
            unsafe {
                WaitForSingleObject(self.handle, INFINITE);
            }
            let reason = WakeReason::from(self.reason.swap(0, Ordering::AcqRel));
            WakeResult::new(reason, None, None)
        }

        fn wait_timeout(&self, timeout: Duration) -> Option<WakeResult> {
            let ms = timeout.as_millis() as u32;
            let result = unsafe { WaitForSingleObject(self.handle, ms) };

            match result {
                WAIT_OBJECT_0 => {
                    let reason = WakeReason::from(self.reason.swap(0, Ordering::AcqRel));
                    Some(WakeResult::new(reason, None, None))
                }
                WAIT_TIMEOUT => None,
                _ => None,
            }
        }

        fn signal(&self, reason: WakeReason, _from_ai: &str, _content: &str) {
            // Store reason before signaling to avoid a race where the waiter
            // reads reason before it is written.
            self.reason.store(reason as u8, Ordering::Release);
            unsafe {
                SetEvent(self.handle);
            }
        }

        fn try_recv(&self) -> Option<WakeResult> {
            self.wait_timeout(Duration::from_millis(0))
        }
    }

    pub type PlatformWakeEvent = WindowsWakeEvent;
}

// ============================================================================
// UNIX IMPLEMENTATION - POSIX named semaphores
// ============================================================================

#[cfg(not(target_os = "windows"))]
pub mod unix {
    use super::*;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

    static ANON_COUNTER: AtomicU64 = AtomicU64::new(1);

    /// Unix wake event using POSIX named semaphores.
    ///
    /// The previous Linux/macOS implementations used process-local primitives
    /// for `open(ai_id)`, so a daemon could "signal" an AI without touching the
    /// semaphore the standby process was waiting on. This type makes the AI ID
    /// part of the OS object name so separate processes use the same wake event.
    pub struct UnixWakeEvent {
        sem: *mut libc::sem_t,
        name: CString,
        unlink_on_drop: bool,
        reason: AtomicU8,
    }

    unsafe impl Send for UnixWakeEvent {}
    unsafe impl Sync for UnixWakeEvent {}

    impl UnixWakeEvent {
        /// Create a unique same-process/cross-thread wake event for tests.
        pub fn new() -> std::io::Result<Self> {
            let n = ANON_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = CString::new(format!("/teamengram_wake_anon_{}_{}", std::process::id(), n))
                .expect("generated semaphore name contains no null bytes");
            Self::open_named(name, true)
        }

        /// Create or open the stable wake event for an AI.
        pub fn open(ai_id: &str) -> std::io::Result<Self> {
            if ai_id.trim().is_empty() || ai_id == "unknown" {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "AI wake event requires a concrete AI ID",
                ));
            }
            Self::open_named(Self::named_ai_sem(ai_id)?, false)
        }

        fn open_named(name: CString, unlink_on_drop: bool) -> std::io::Result<Self> {
            if unlink_on_drop {
                unsafe {
                    libc::sem_unlink(name.as_ptr());
                }
            }

            let sem = unsafe {
                libc::sem_open(name.as_ptr(), libc::O_CREAT, 0o600, 0u32)
            };
            if sem == libc::SEM_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            Ok(Self {
                sem,
                name,
                unlink_on_drop,
                reason: AtomicU8::new(0),
            })
        }

        fn named_ai_sem(ai_id: &str) -> std::io::Result<CString> {
            let mut hash: u64 = 0xcbf29ce484222325u64;
            let mut clean = String::new();
            for byte in ai_id.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x00000100000001b3u64);

                let ch = byte as char;
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    if clean.len() < 48 {
                        clean.push(ch);
                    }
                } else if clean.len() < 48 {
                    clean.push('_');
                }
            }
            if clean.is_empty() {
                clean.push_str("invalid");
            }

            CString::new(format!("/teamengram_wake_{}_{}", clean, hash))
                .map_err(|_| std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "AI ID contains a null byte",
                ))
        }

        fn absolute_timeout(timeout: Duration) -> libc::timespec {
            let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
            unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
            ts.tv_sec += timeout.as_secs() as libc::time_t;
            ts.tv_nsec += timeout.subsec_nanos() as libc::c_long;
            while ts.tv_nsec >= 1_000_000_000 {
                ts.tv_sec += 1;
                ts.tv_nsec -= 1_000_000_000;
            }
            ts
        }
    }

    impl Drop for UnixWakeEvent {
        fn drop(&mut self) {
            unsafe {
                libc::sem_close(self.sem);
                if self.unlink_on_drop {
                    libc::sem_unlink(self.name.as_ptr());
                }
            }
        }
    }

    impl WakeEvent for UnixWakeEvent {
        fn wait(&self) -> WakeResult {
            loop {
                let result = unsafe { libc::sem_wait(self.sem) };
                if result == 0 {
                    let reason = WakeReason::from(self.reason.swap(0, Ordering::AcqRel));
                    return WakeResult::new(reason, None, None);
                }

                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                panic!("wake wait failed for {:?}: {}", self.name, err);
            }
        }

        fn wait_timeout(&self, timeout: Duration) -> Option<WakeResult> {
            let ts = Self::absolute_timeout(timeout);
            loop {
                let result = unsafe { libc::sem_timedwait(self.sem, &ts) };
                if result == 0 {
                    let reason = WakeReason::from(self.reason.swap(0, Ordering::AcqRel));
                    return Some(WakeResult::new(reason, None, None));
                }

                let err = std::io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(libc::ETIMEDOUT) => return None,
                    Some(libc::EINTR) => continue,
                    _ => panic!("wake timed wait failed for {:?}: {}", self.name, err),
                }
            }
        }

        fn signal(&self, reason: WakeReason, _from_ai: &str, _content: &str) {
            self.reason.store(reason as u8, Ordering::Release);
            let result = unsafe { libc::sem_post(self.sem) };
            if result != 0 {
                panic!("wake signal failed for {:?}: {}", self.name, std::io::Error::last_os_error());
            }
        }

        fn try_recv(&self) -> Option<WakeResult> {
            loop {
                let result = unsafe { libc::sem_trywait(self.sem) };
                if result == 0 {
                    let reason = WakeReason::from(self.reason.swap(0, Ordering::AcqRel));
                    return Some(WakeResult::new(reason, None, None));
                }

                let err = std::io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(libc::EAGAIN) => return None,
                    Some(libc::EINTR) => continue,
                    _ => panic!("wake try_recv failed for {:?}: {}", self.name, err),
                }
            }
        }
    }

    pub type PlatformWakeEvent = UnixWakeEvent;
}

// ============================================================================
// PLATFORM EXPORT
// ============================================================================

#[cfg(target_os = "windows")]
pub use windows::PlatformWakeEvent;

#[cfg(not(target_os = "windows"))]
pub use unix::PlatformWakeEvent;

// ============================================================================
// CROSS-PROCESS WAKE COORDINATOR
// ============================================================================

use anyhow::Result;

/// Manages wake events for multiple AIs via shared memory
pub struct WakeCoordinator {
    /// AI ID this coordinator is for
    ai_id: String,
    /// Platform-specific wake event
    event: PlatformWakeEvent,
}

impl WakeCoordinator {
    /// Create wake coordinator for an AI
    pub fn new(ai_id: &str) -> Result<Self> {
        let event = PlatformWakeEvent::open(ai_id)
            .map_err(|e| anyhow::anyhow!("Failed to create wake event: {}", e))?;

        Ok(Self {
            ai_id: ai_id.to_string(),
            event,
        })
    }

    /// Wait for wake event (blocking)
    pub fn wait(&self) -> WakeResult {
        self.event.wait()
    }

    /// Wait with timeout
    pub fn wait_timeout(&self, timeout: Duration) -> Option<WakeResult> {
        self.event.wait_timeout(timeout)
    }

    /// Signal this AI to wake up
    pub fn wake(&self, reason: WakeReason, from_ai: &str, content: &str) {
        self.event.signal(reason, from_ai, content);
    }

    /// Check for wake without blocking
    pub fn try_recv(&self) -> Option<WakeResult> {
        self.event.try_recv()
    }

    /// Get AI ID
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }
}

// ============================================================================
// PRESENCE DETECTION - OS-level mutex that auto-releases on process death
// NO POLLING, NO TTL - pure event-driven using OS primitives
// ============================================================================

/// Presence mutex - held while daemon is alive, auto-released on death
#[cfg(target_os = "windows")]
pub struct PresenceMutex {
    handle: *mut std::ffi::c_void,
    ai_id: String,
}

#[cfg(target_os = "windows")]
unsafe impl Send for PresenceMutex {}
#[cfg(target_os = "windows")]
unsafe impl Sync for PresenceMutex {}

#[cfg(target_os = "windows")]
impl PresenceMutex {
    /// Create a presence mutex for this AI. Held until Drop or process death.
    pub fn acquire(ai_id: &str) -> std::io::Result<Self> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;

        #[link(name = "kernel32")]
        extern "system" {
            fn CreateMutexW(
                lpMutexAttributes: *mut std::ffi::c_void,
                bInitialOwner: i32,
                lpName: *const u16,
            ) -> *mut std::ffi::c_void;
        }

        let name = format!(r"Local\TeamEngram_Alive_{}", ai_id);
        let wide_name: Vec<u16> = OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            CreateMutexW(ptr::null_mut(), 1, wide_name.as_ptr()) // 1 = take ownership
        };

        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            handle,
            ai_id: ai_id.to_string(),
        })
    }

    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }
}

#[cfg(target_os = "windows")]
impl Drop for PresenceMutex {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn ReleaseMutex(hMutex: *mut std::ffi::c_void) -> i32;
            fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
        }

        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

/// Check if an AI is online by testing if their presence mutex exists
#[cfg(target_os = "windows")]
pub fn is_ai_online(ai_id: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenMutexW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }

    const SYNCHRONIZE: u32 = 0x00100000;

    let name = format!(r"Local\TeamEngram_Alive_{}", ai_id);
    let wide_name: Vec<u16> = OsStr::new(&name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, wide_name.as_ptr()) };

    if handle.is_null() {
        false // Mutex doesn't exist = AI is offline
    } else {
        unsafe { CloseHandle(handle) };
        true // Mutex exists = AI is online
    }
}

/// Get list of online AIs by checking known AI IDs
#[cfg(target_os = "windows")]
pub fn get_online_ais(known_ai_ids: &[&str]) -> Vec<String> {
    known_ai_ids
        .iter()
        .filter(|id| is_ai_online(id))
        .map(|s| s.to_string())
        .collect()
}

// Unix presence via PID files
#[cfg(not(target_os = "windows"))]
pub struct PresenceMutex {
    ai_id: String,
    pid_path: std::path::PathBuf,
}

#[cfg(not(target_os = "windows"))]
impl PresenceMutex {
    pub fn acquire(ai_id: &str) -> std::io::Result<Self> {
        let presence_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".ai-foundation")
            .join("v2")
            .join("presence");
        std::fs::create_dir_all(&presence_dir)?;

        let pid_path = presence_dir.join(format!("{}.pid", ai_id));
        std::fs::write(&pid_path, format!("{}", std::process::id()))?;

        Ok(Self {
            ai_id: ai_id.to_string(),
            pid_path,
        })
    }
    pub fn ai_id(&self) -> &str { &self.ai_id }
}

#[cfg(not(target_os = "windows"))]
impl Drop for PresenceMutex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.pid_path);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_ai_online(ai_id: &str) -> bool {
    // Check for PID file at well-known location
    let pid_path = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".ai-foundation")
        .join("v2")
        .join("presence")
        .join(format!("{}.pid", ai_id));

    if let Ok(contents) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            // Check if process is still alive (signal 0 = existence check)
            unsafe { libc::kill(pid, 0) == 0 }
        } else {
            false
        }
    } else {
        false
    }
}

#[cfg(not(target_os = "windows"))]
pub fn get_online_ais(_known_ai_ids: &[&str]) -> Vec<String> {
    vec![]
}

// ============================================================================
// SEQUENCER WAKE EVENT - Cross-process signal for instant event processing
// ============================================================================

/// Lightweight cross-process wake event for the Sequencer daemon.
///
/// Writers (AIs) signal this after writing to their outbox.
/// The Sequencer waits on this instead of polling/timeout.
///
/// Uses Windows Named Events - ~500ns signal, ~1us wake, zero CPU while waiting.
#[cfg(target_os = "windows")]
pub mod sequencer_wake {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::time::Duration;

    const WAIT_OBJECT_0: u32 = 0;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x00100000;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lpEventAttributes: *mut std::ffi::c_void,
            bManualReset: i32,
            bInitialState: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;

        fn OpenEventW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;

        fn SetEvent(hEvent: *mut std::ffi::c_void) -> i32;
        fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }

    /// Compute per-data-dir suffix for the sequencer wake event name.
    /// Uses FNV-1a hash of the canonical path — same approach as the singleton mutex.
    fn sequencer_event_suffix(base_dir: Option<&std::path::Path>) -> String {
        if let Some(dir) = base_dir {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            let path_str = canonical.to_string_lossy();
            let mut hash: u64 = 0xcbf29ce484222325u64;
            for byte in path_str.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x00000100000001b3u64);
            }
            format!("_{:016x}", hash)
        } else {
            String::new()
        }
    }

    /// Sequencer-side: waits for signals from outbox writers
    pub struct SequencerWakeReceiver {
        handle: *mut std::ffi::c_void,
    }

    unsafe impl Send for SequencerWakeReceiver {}
    unsafe impl Sync for SequencerWakeReceiver {}

    impl SequencerWakeReceiver {
        /// Create or open the sequencer wake event (receiver side).
        /// `base_dir` makes the event name unique per data directory so that
        /// multiple daemon instances (e.g. production + test) do not share the
        /// same event and steal each other's wake signals.
        pub fn new(base_dir: Option<&std::path::Path>) -> std::io::Result<Self> {
            let event_name = format!(r"Local\TeamEngram_SequencerWake{}", sequencer_event_suffix(base_dir));
            let wide_name: Vec<u16> = OsStr::new(&event_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            // Try to open existing event first
            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE | SYNCHRONIZE,
                    0,
                    wide_name.as_ptr(),
                )
            };

            let handle = if handle.is_null() {
                // Create new auto-reset event
                let h = unsafe {
                    CreateEventW(
                        ptr::null_mut(),
                        0, // Auto-reset: resets after one waiter is released
                        0, // Initially non-signaled
                        wide_name.as_ptr(),
                    )
                };

                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                h
            } else {
                handle
            };

            Ok(Self { handle })
        }

        /// Wait for a signal with timeout. Returns true if signaled, false if timeout.
        pub fn wait_timeout(&self, timeout: Duration) -> bool {
            let ms = timeout.as_millis() as u32;
            let result = unsafe { WaitForSingleObject(self.handle, ms) };
            result == WAIT_OBJECT_0
        }

        /// Block until signaled. No timeout. No polling.
        ///
        /// Uses WaitForSingleObject(INFINITE). Wakes ONLY when:
        /// - An outbox write calls signal_sequencer() (normal path)
        /// - The shutdown handler calls signal_sequencer() (clean exit)
        ///
        /// If the signal mechanism is broken, this blocks forever.
        /// That is correct behavior — fix the signal, don't mask the bug.
        pub fn wait(&self) {
            unsafe { WaitForSingleObject(self.handle, 0xFFFFFFFF) };
        }
    }

    impl Drop for SequencerWakeReceiver {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Writer-side: signals the sequencer after writing to outbox
    pub struct SequencerWakeSignaler {
        handle: *mut std::ffi::c_void,
    }

    unsafe impl Send for SequencerWakeSignaler {}
    unsafe impl Sync for SequencerWakeSignaler {}

    impl SequencerWakeSignaler {
        /// Open connection to the sequencer wake event (signaler side)
        ///
        /// Returns None if sequencer is not running (event doesn't exist).
        /// This is expected when no daemon is active.
        pub fn open(base_dir: Option<&std::path::Path>) -> Option<Self> {
            let event_name = format!(r"Local\TeamEngram_SequencerWake{}", sequencer_event_suffix(base_dir));
            let wide_name: Vec<u16> = OsStr::new(&event_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE,
                    0,
                    wide_name.as_ptr(),
                )
            };

            if handle.is_null() {
                None // Sequencer not running
            } else {
                Some(Self { handle })
            }
        }

        /// Signal the sequencer that new events are available
        pub fn signal(&self) {
            unsafe { SetEvent(self.handle) };
        }
    }

    impl Drop for SequencerWakeSignaler {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Convenience function: signal sequencer if running (fire-and-forget).
    ///
    /// `base_dir` must match what the daemon was started with so the signal
    /// reaches the correct daemon instance (not the production daemon when
    /// called from a test).
    pub fn signal_sequencer(base_dir: Option<&std::path::Path>) {
        if let Some(signaler) = SequencerWakeSignaler::open(base_dir) {
            signaler.signal();
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod sequencer_wake {
    use std::time::Duration;
    use std::ffi::CString;

    /// Compute per-data-dir suffix for the sequencer wake semaphore name.
    /// Uses FNV-1a hash of the canonical path — same approach as the singleton mutex.
    fn sequencer_event_suffix(base_dir: Option<&std::path::Path>) -> String {
        if let Some(dir) = base_dir {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            let path_str = canonical.to_string_lossy();
            let mut hash: u64 = 0xcbf29ce484222325u64;
            for byte in path_str.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x00000100000001b3u64);
            }
            format!("_{:016x}", hash)
        } else {
            String::new()
        }
    }

    fn sem_name(base_dir: Option<&std::path::Path>) -> CString {
        CString::new(format!("/teamengram_seq_wake{}", sequencer_event_suffix(base_dir)))
            .expect("sem name contains no null bytes")
    }

    /// Cross-process sequencer wake using POSIX named semaphores.
    ///
    /// Named semaphores persist in /dev/shm/ and are visible across all processes
    /// on the same host. This enables true cross-process signaling on Linux/macOS
    /// with zero polling — sem_post/sem_timedwait are ~200ns on modern kernels.
    pub struct SequencerWakeReceiver {
        sem: *mut libc::sem_t,
        name: CString,
    }

    unsafe impl Send for SequencerWakeReceiver {}
    unsafe impl Sync for SequencerWakeReceiver {}

    impl SequencerWakeReceiver {
        /// `base_dir` makes the semaphore name unique per data directory so that
        /// multiple daemon instances (e.g. production + test) do not share the
        /// same semaphore and steal each other's wake signals.
        pub fn new(base_dir: Option<&std::path::Path>) -> std::io::Result<Self> {
            let name = sem_name(base_dir);
            let sem = unsafe {
                libc::sem_open(
                    name.as_ptr(),
                    libc::O_CREAT,
                    0o644,
                    0u32,  // Initially non-signaled
                )
            };
            if sem == libc::SEM_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            // Drain any stale signals from previous runs
            loop {
                let result = unsafe { libc::sem_trywait(sem) };
                if result != 0 { break; }
            }

            Ok(Self { sem, name })
        }

        pub fn wait_timeout(&self, timeout: Duration) -> bool {
            let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
            unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
            ts.tv_sec += timeout.as_secs() as libc::time_t;
            ts.tv_nsec += timeout.subsec_nanos() as libc::c_long;
            if ts.tv_nsec >= 1_000_000_000 {
                ts.tv_sec += 1;
                ts.tv_nsec -= 1_000_000_000;
            }

            let result = unsafe { libc::sem_timedwait(self.sem, &ts) };
            result == 0
        }

        /// Block until signaled. No timeout. No polling.
        ///
        /// Uses sem_wait (POSIX). Wakes ONLY when:
        /// - An outbox write calls signal_sequencer() → sem_post (normal path)
        /// - The shutdown handler calls signal_sequencer() → sem_post (clean exit)
        /// - A signal (SIGINT) interrupts sem_wait with EINTR (also triggers exit)
        ///
        /// On EINTR, we return to let the caller check stop_signal.
        /// If the signal mechanism is broken, this blocks forever.
        /// That is correct behavior — fix the signal, don't mask the bug.
        pub fn wait(&self) {
            unsafe { libc::sem_wait(self.sem) };
            // Return value unchecked: whether signaled (0) or interrupted (EINTR),
            // the caller's loop checks stop_signal and processes events regardless.
        }
    }

    impl Drop for SequencerWakeReceiver {
        fn drop(&mut self) {
            unsafe {
                libc::sem_close(self.sem);
                // Unlink so it doesn't persist after daemon shutdown
                libc::sem_unlink(self.name.as_ptr());
            }
        }
    }

    pub struct SequencerWakeSignaler {
        sem: *mut libc::sem_t,
    }

    unsafe impl Send for SequencerWakeSignaler {}
    unsafe impl Sync for SequencerWakeSignaler {}

    impl SequencerWakeSignaler {
        /// Open connection to the sequencer wake semaphore.
        /// Returns None if sequencer is not running (semaphore doesn't exist).
        pub fn open(base_dir: Option<&std::path::Path>) -> Option<Self> {
            let name = sem_name(base_dir);
            let sem = unsafe {
                libc::sem_open(
                    name.as_ptr(),
                    0,  // Open existing only, don't create
                    0,
                    0,
                )
            };
            if sem == libc::SEM_FAILED {
                None  // Sequencer not running
            } else {
                Some(Self { sem })
            }
        }

        pub fn signal(&self) {
            unsafe { libc::sem_post(self.sem) };
        }
    }

    impl Drop for SequencerWakeSignaler {
        fn drop(&mut self) {
            unsafe { libc::sem_close(self.sem) };
        }
    }

    /// Convenience function: signal sequencer if running (fire-and-forget).
    ///
    /// `base_dir` must match what the daemon was started with so the signal
    /// reaches the correct daemon instance (not the production daemon when
    /// called from a test).
    pub fn signal_sequencer(base_dir: Option<&std::path::Path>) {
        if let Some(signaler) = SequencerWakeSignaler::open(base_dir) {
            signaler.signal();
        }
    }
}

// Re-export for convenience
pub use sequencer_wake::{SequencerWakeReceiver, SequencerWakeSignaler, signal_sequencer};

// ============================================================================
// FEDERATION WAKE — signals federation node when new events hit the master log
// ============================================================================
//
// Same pattern as SequencerWake: OS-native wait primitive, zero polling.
// The sequencer calls signal_federation() after writing events to the master
// event log. The federation node blocks on FederationWakeReceiver::wait()
// and wakes instantly (~1μs) to read and push new events to peers.

#[cfg(target_os = "windows")]
pub mod federation_wake {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::time::Duration;

    const WAIT_OBJECT_0: u32 = 0;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x00100000;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lpEventAttributes: *mut std::ffi::c_void,
            bManualReset: i32,
            bInitialState: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn OpenEventW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn SetEvent(hEvent: *mut std::ffi::c_void) -> i32;
        fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }

    fn federation_event_suffix(base_dir: Option<&std::path::Path>) -> String {
        if let Some(dir) = base_dir {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            let path_str = canonical.to_string_lossy();
            let mut hash: u64 = 0xcbf29ce484222325u64;
            for byte in path_str.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x00000100000001b3u64);
            }
            format!("_{:016x}", hash)
        } else {
            String::new()
        }
    }

    /// Federation-side: blocks until the sequencer signals new events in the master log.
    pub struct FederationWakeReceiver {
        handle: *mut std::ffi::c_void,
    }

    unsafe impl Send for FederationWakeReceiver {}
    unsafe impl Sync for FederationWakeReceiver {}

    impl FederationWakeReceiver {
        pub fn new(base_dir: Option<&std::path::Path>) -> std::io::Result<Self> {
            let event_name = format!(r"Local\TeamEngram_FederationWake{}", federation_event_suffix(base_dir));
            let wide_name: Vec<u16> = OsStr::new(&event_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE | SYNCHRONIZE,
                    0,
                    wide_name.as_ptr(),
                )
            };

            let handle = if handle.is_null() {
                let h = unsafe {
                    CreateEventW(
                        ptr::null_mut(),
                        0, // Auto-reset
                        0, // Initially non-signaled
                        wide_name.as_ptr(),
                    )
                };
                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                h
            } else {
                handle
            };

            Ok(Self { handle })
        }

        /// Block until signaled. No timeout. No polling.
        ///
        /// Wakes ONLY when the sequencer writes new events to the master log
        /// and calls signal_federation(). If the signal is broken, this blocks
        /// forever. That is correct — fix the signal, don't mask the bug.
        pub fn wait(&self) {
            unsafe { WaitForSingleObject(self.handle, 0xFFFFFFFF) };
        }

        /// Wait with timeout. Returns true if signaled, false if timeout.
        pub fn wait_timeout(&self, timeout: Duration) -> bool {
            let ms = timeout.as_millis() as u32;
            let result = unsafe { WaitForSingleObject(self.handle, ms) };
            result == WAIT_OBJECT_0
        }
    }

    impl Drop for FederationWakeReceiver {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Sequencer-side: signals the federation node after writing to the master log.
    pub struct FederationWakeSignaler {
        handle: *mut std::ffi::c_void,
    }

    unsafe impl Send for FederationWakeSignaler {}
    unsafe impl Sync for FederationWakeSignaler {}

    impl FederationWakeSignaler {
        /// Open connection to the federation wake event.
        /// Returns None if federation node is not running (event doesn't exist).
        pub fn open(base_dir: Option<&std::path::Path>) -> Option<Self> {
            let event_name = format!(r"Local\TeamEngram_FederationWake{}", federation_event_suffix(base_dir));
            let wide_name: Vec<u16> = OsStr::new(&event_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE,
                    0,
                    wide_name.as_ptr(),
                )
            };

            if handle.is_null() {
                None // Federation node not running
            } else {
                Some(Self { handle })
            }
        }

        pub fn signal(&self) {
            unsafe { SetEvent(self.handle) };
        }
    }

    impl Drop for FederationWakeSignaler {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Convenience: signal federation node if running (fire-and-forget).
    pub fn signal_federation(base_dir: Option<&std::path::Path>) {
        if let Some(signaler) = FederationWakeSignaler::open(base_dir) {
            signaler.signal();
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod federation_wake {
    use std::time::Duration;
    use std::ffi::CString;

    fn federation_event_suffix(base_dir: Option<&std::path::Path>) -> String {
        if let Some(dir) = base_dir {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            let path_str = canonical.to_string_lossy();
            let mut hash: u64 = 0xcbf29ce484222325u64;
            for byte in path_str.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x00000100000001b3u64);
            }
            format!("_{:016x}", hash)
        } else {
            String::new()
        }
    }

    fn sem_name(base_dir: Option<&std::path::Path>) -> CString {
        CString::new(format!("/teamengram_fed_wake{}", federation_event_suffix(base_dir)))
            .expect("sem name contains no null bytes")
    }

    /// Federation-side: blocks until the sequencer signals new events.
    pub struct FederationWakeReceiver {
        sem: *mut libc::sem_t,
        name: CString,
    }

    unsafe impl Send for FederationWakeReceiver {}
    unsafe impl Sync for FederationWakeReceiver {}

    impl FederationWakeReceiver {
        pub fn new(base_dir: Option<&std::path::Path>) -> std::io::Result<Self> {
            let name = sem_name(base_dir);
            let sem = unsafe {
                libc::sem_open(
                    name.as_ptr(),
                    libc::O_CREAT,
                    0o644,
                    0u32,
                )
            };
            if sem == libc::SEM_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            // Drain stale signals
            loop {
                let result = unsafe { libc::sem_trywait(sem) };
                if result != 0 { break; }
            }

            Ok(Self { sem, name })
        }

        /// Block until signaled. No timeout. No polling.
        pub fn wait(&self) {
            unsafe { libc::sem_wait(self.sem) };
        }

        /// Wait with timeout. Returns true if signaled, false if timeout.
        pub fn wait_timeout(&self, timeout: Duration) -> bool {
            let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
            unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
            ts.tv_sec += timeout.as_secs() as libc::time_t;
            ts.tv_nsec += timeout.subsec_nanos() as libc::c_long;
            if ts.tv_nsec >= 1_000_000_000 {
                ts.tv_sec += 1;
                ts.tv_nsec -= 1_000_000_000;
            }
            let result = unsafe { libc::sem_timedwait(self.sem, &ts) };
            result == 0
        }
    }

    impl Drop for FederationWakeReceiver {
        fn drop(&mut self) {
            unsafe {
                libc::sem_close(self.sem);
                libc::sem_unlink(self.name.as_ptr());
            }
        }
    }

    pub struct FederationWakeSignaler {
        sem: *mut libc::sem_t,
    }

    unsafe impl Send for FederationWakeSignaler {}
    unsafe impl Sync for FederationWakeSignaler {}

    impl FederationWakeSignaler {
        pub fn open(base_dir: Option<&std::path::Path>) -> Option<Self> {
            let name = sem_name(base_dir);
            let sem = unsafe {
                libc::sem_open(
                    name.as_ptr(),
                    0,
                    0,
                    0,
                )
            };
            if sem == libc::SEM_FAILED {
                None
            } else {
                Some(Self { sem })
            }
        }

        pub fn signal(&self) {
            unsafe { libc::sem_post(self.sem) };
        }
    }

    impl Drop for FederationWakeSignaler {
        fn drop(&mut self) {
            unsafe { libc::sem_close(self.sem) };
        }
    }

    /// Convenience: signal federation node if running (fire-and-forget).
    pub fn signal_federation(base_dir: Option<&std::path::Path>) {
        if let Some(signaler) = FederationWakeSignaler::open(base_dir) {
            signaler.signal();
        }
    }
}

pub use federation_wake::{FederationWakeReceiver, FederationWakeSignaler, signal_federation};

// ============================================================================
// OUTBOX DRAIN WAKE — signals writer when sequencer has drained their outbox
// ============================================================================
//
// Replaces sleep-based backpressure retry with event-driven waiting.
// When a writer's outbox is full:
//   1. Writer signals sequencer (existing signal_sequencer)
//   2. Writer blocks on DrainWakeReceiver::wait_timeout() — zero CPU
//   3. Sequencer drains events, calls signal_outbox_drained(ai_id)
//   4. Writer wakes instantly (~1μs), checks space, writes or retries
//
// Per-AI events: each outbox has its own drain event (ai_id in name).

#[cfg(target_os = "windows")]
pub mod outbox_drain_wake {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::time::Duration;

    const WAIT_OBJECT_0: u32 = 0;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x00100000;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lpEventAttributes: *mut std::ffi::c_void,
            bManualReset: i32,
            bInitialState: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn OpenEventW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn SetEvent(hEvent: *mut std::ffi::c_void) -> i32;
        fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }

    /// Compute per-outbox event name suffix from ai_id + base_dir.
    fn drain_event_name(ai_id: &str, base_dir: Option<&std::path::Path>) -> String {
        let dir_str = base_dir.map_or("default".to_string(), |p| {
            p.canonicalize().unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_string()
        });
        let combined = format!("{}/{}", dir_str, ai_id);
        let mut hash: u64 = 0xcbf29ce484222325u64;
        for byte in combined.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x00000100000001b3u64);
        }
        format!(r"Local\TeamEngram_OutboxDrain_{:016x}", hash)
    }

    /// Writer-side: waits for sequencer to drain the outbox.
    /// Created by OutboxProducer, stored as a field.
    pub struct DrainWakeReceiver {
        handle: *mut std::ffi::c_void,
    }

    unsafe impl Send for DrainWakeReceiver {}
    unsafe impl Sync for DrainWakeReceiver {}

    impl DrainWakeReceiver {
        /// Create or open the drain event for this AI's outbox.
        pub fn new(ai_id: &str, base_dir: Option<&std::path::Path>) -> std::io::Result<Self> {
            let event_name = drain_event_name(ai_id, base_dir);
            let wide_name: Vec<u16> = OsStr::new(&event_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE | SYNCHRONIZE,
                    0,
                    wide_name.as_ptr(),
                )
            };

            let handle = if handle.is_null() {
                let h = unsafe {
                    CreateEventW(
                        ptr::null_mut(),
                        0, // Auto-reset
                        0, // Initially non-signaled
                        wide_name.as_ptr(),
                    )
                };
                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                h
            } else {
                handle
            };

            Ok(Self { handle })
        }

        /// Wait for drain signal with timeout. Returns true if signaled, false if timeout.
        /// Zero CPU while waiting — blocked on WaitForSingleObject.
        pub fn wait_timeout(&self, timeout: Duration) -> bool {
            let ms = timeout.as_millis() as u32;
            let result = unsafe { WaitForSingleObject(self.handle, ms) };
            result == WAIT_OBJECT_0
        }
    }

    impl Drop for DrainWakeReceiver {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Sequencer-side: signals that an outbox has been drained.
    pub fn signal_outbox_drained(ai_id: &str, base_dir: Option<&std::path::Path>) {
        let event_name = drain_event_name(ai_id, base_dir);
        let wide_name: Vec<u16> = OsStr::new(&event_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            OpenEventW(
                EVENT_MODIFY_STATE,
                0,
                wide_name.as_ptr(),
            )
        };

        if !handle.is_null() {
            unsafe {
                SetEvent(handle);
                CloseHandle(handle);
            }
        }
        // If handle is null, no writer is waiting — that's fine.
    }
}

#[cfg(not(target_os = "windows"))]
pub mod outbox_drain_wake {
    use std::time::Duration;
    use std::ffi::CString;

    /// Compute per-outbox semaphore name from ai_id + base_dir.
    fn drain_sem_name(ai_id: &str, base_dir: Option<&std::path::Path>) -> CString {
        let dir_str = base_dir.map_or("default".to_string(), |p| {
            p.canonicalize().unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_string()
        });
        let combined = format!("{}/{}", dir_str, ai_id);
        let mut hash: u64 = 0xcbf29ce484222325u64;
        for byte in combined.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x00000100000001b3u64);
        }
        CString::new(format!("/te_drain_{:016x}", hash))
            .expect("sem name contains no null bytes")
    }

    /// Writer-side: waits for sequencer to drain the outbox.
    pub struct DrainWakeReceiver {
        sem: *mut libc::sem_t,
        name: CString,
    }

    unsafe impl Send for DrainWakeReceiver {}
    unsafe impl Sync for DrainWakeReceiver {}

    impl DrainWakeReceiver {
        pub fn new(ai_id: &str, base_dir: Option<&std::path::Path>) -> std::io::Result<Self> {
            let name = drain_sem_name(ai_id, base_dir);
            let sem = unsafe {
                libc::sem_open(
                    name.as_ptr(),
                    libc::O_CREAT,
                    0o644,
                    0u32,
                )
            };
            if sem == libc::SEM_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            // Drain stale signals
            loop {
                let result = unsafe { libc::sem_trywait(sem) };
                if result != 0 { break; }
            }

            Ok(Self { sem, name })
        }

        /// Wait for drain signal with timeout. Returns true if signaled, false if timeout.
        pub fn wait_timeout(&self, timeout: Duration) -> bool {
            let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
            unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
            ts.tv_sec += timeout.as_secs() as libc::time_t;
            ts.tv_nsec += timeout.subsec_nanos() as libc::c_long;
            if ts.tv_nsec >= 1_000_000_000 {
                ts.tv_sec += 1;
                ts.tv_nsec -= 1_000_000_000;
            }
            let result = unsafe { libc::sem_timedwait(self.sem, &ts) };
            result == 0
        }
    }

    impl Drop for DrainWakeReceiver {
        fn drop(&mut self) {
            unsafe {
                libc::sem_close(self.sem);
                libc::sem_unlink(self.name.as_ptr());
            }
        }
    }

    /// Sequencer-side: signals that an outbox has been drained.
    pub fn signal_outbox_drained(ai_id: &str, base_dir: Option<&std::path::Path>) {
        let name = drain_sem_name(ai_id, base_dir);
        let sem = unsafe {
            libc::sem_open(
                name.as_ptr(),
                0,
                0,
                0,
            )
        };
        if sem != libc::SEM_FAILED {
            unsafe {
                libc::sem_post(sem);
                libc::sem_close(sem);
            }
        }
    }
}

pub use outbox_drain_wake::{DrainWakeReceiver, signal_outbox_drained};

// ============================================================================
// DAEMON READY WAKE — signals clients when daemon pipe listener is ready
// ============================================================================
//
// Replaces sleep-based startup backoff with event-driven readiness.
// Daemon signals after creating the named pipe listener.
// Client blocks on DaemonReadyReceiver::wait_timeout() — zero CPU.

#[cfg(target_os = "windows")]
pub mod daemon_ready_wake {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::time::Duration;

    const WAIT_OBJECT_0: u32 = 0;
    const SYNCHRONIZE: u32 = 0x00100000;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lpEventAttributes: *mut std::ffi::c_void,
            bManualReset: i32,
            bInitialState: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn OpenEventW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }

    fn ready_event_name(ai_id: &str) -> String {
        // Per-AI daemon ready event (each AI has its own daemon)
        let mut hash: u64 = 0xcbf29ce484222325u64;
        for byte in ai_id.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x00000100000001b3u64);
        }
        format!(r"Local\TeamEngram_DaemonReady_{:016x}", hash)
    }

    /// Daemon-side: signals readiness after pipe listener is created.
    pub fn signal_daemon_ready(ai_id: &str) {
        let event_name = ready_event_name(ai_id);
        let wide_name: Vec<u16> = OsStr::new(&event_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Create manual-reset event (so ALL waiting clients wake up)
        // bInitialState=1: already signaled on creation = ready NOW.
        let _handle = unsafe {
            CreateEventW(
                ptr::null_mut(),
                1, // Manual-reset: stays signaled until explicitly reset
                1, // Initially signaled (we're ready NOW)
                wide_name.as_ptr(),
            )
        };
        // Handle intentionally leaked — event stays alive for daemon process lifetime.
        // Kernel cleans up all handles on process exit.
    }

    /// Client-side: waits for daemon to be ready.
    pub fn wait_daemon_ready(ai_id: &str, timeout: Duration) -> bool {
        let event_name = ready_event_name(ai_id);
        let wide_name: Vec<u16> = OsStr::new(&event_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            OpenEventW(
                SYNCHRONIZE,
                0,
                wide_name.as_ptr(),
            )
        };

        if handle.is_null() {
            return false; // Event doesn't exist yet — daemon hasn't created it
        }

        let ms = timeout.as_millis() as u32;
        let result = unsafe { WaitForSingleObject(handle, ms) };
        unsafe { CloseHandle(handle) };
        result == WAIT_OBJECT_0
    }
}

#[cfg(not(target_os = "windows"))]
pub mod daemon_ready_wake {
    use std::time::Duration;
    use std::ffi::CString;

    fn ready_sem_name(ai_id: &str) -> CString {
        let mut hash: u64 = 0xcbf29ce484222325u64;
        for byte in ai_id.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x00000100000001b3u64);
        }
        CString::new(format!("/te_ready_{:016x}", hash))
            .expect("sem name contains no null bytes")
    }

    /// Daemon-side: signals readiness.
    pub fn signal_daemon_ready(ai_id: &str) {
        let name = ready_sem_name(ai_id);
        let sem = unsafe {
            libc::sem_open(
                name.as_ptr(),
                libc::O_CREAT,
                0o644,
                1u32, // Initially signaled (ready)
            )
        };
        if sem != libc::SEM_FAILED {
            unsafe { libc::sem_post(sem) };
            // Don't close or unlink — stays alive for daemon lifetime.
        }
    }

    /// Client-side: waits for daemon to be ready.
    pub fn wait_daemon_ready(ai_id: &str, timeout: Duration) -> bool {
        let name = ready_sem_name(ai_id);
        let sem = unsafe {
            libc::sem_open(
                name.as_ptr(),
                0,
                0,
                0,
            )
        };
        if sem == libc::SEM_FAILED {
            return false;
        }

        let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
        unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
        ts.tv_sec += timeout.as_secs() as libc::time_t;
        ts.tv_nsec += timeout.subsec_nanos() as libc::c_long;
        if ts.tv_nsec >= 1_000_000_000 {
            ts.tv_sec += 1;
            ts.tv_nsec -= 1_000_000_000;
        }

        let result = unsafe { libc::sem_timedwait(sem, &ts) };
        unsafe { libc::sem_close(sem) };
        result == 0
    }
}

pub use daemon_ready_wake::{signal_daemon_ready, wait_daemon_ready};

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Instant;

    #[test]
    fn test_wake_event_signal() {
        let event = PlatformWakeEvent::new().unwrap();

        // Signal should not block
        event.signal(WakeReason::DirectMessage, "test-ai", "hello");

        // Should receive immediately
        let result = event.wait_timeout(Duration::from_millis(100));
        assert!(result.is_some());
        assert_eq!(result.unwrap().reason, WakeReason::DirectMessage);
    }

    #[test]
    fn test_wake_event_timeout() {
        let event = PlatformWakeEvent::new().unwrap();

        let start = Instant::now();
        let result = event.wait_timeout(Duration::from_millis(50));
        let elapsed = start.elapsed();

        assert!(result.is_none());
        assert!(elapsed >= Duration::from_millis(45)); // Allow some slack
        assert!(elapsed < Duration::from_millis(100));
    }

    #[test]
    fn test_wake_coordinator() {
        let coord = WakeCoordinator::new("test-ai-123").unwrap();

        coord.wake(WakeReason::Urgent, "other-ai", "urgent message");

        let result = coord.wait_timeout(Duration::from_millis(100));
        assert!(result.is_some());
        assert_eq!(result.unwrap().reason, WakeReason::Urgent);
    }

    #[test]
    fn test_cross_thread_wake() {
        use std::sync::Arc;

        let event = Arc::new(PlatformWakeEvent::new().unwrap());
        let event_clone = event.clone();

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            event_clone.signal(WakeReason::TaskAssigned, "worker", "task ready");
        });

        let start = Instant::now();
        let result = event.wait_timeout(Duration::from_millis(200));
        let elapsed = start.elapsed();

        handle.join().unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().reason, WakeReason::TaskAssigned);
        assert!(elapsed >= Duration::from_millis(40));
        assert!(elapsed < Duration::from_millis(150));
    }
}
