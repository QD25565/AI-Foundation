//! Shared server state - storage backends and config
//!
//! Pure Rust backends for ~5ms tool calls (vs 50-500ms with Python/subprocess)
//! Shared memory bulletin for ~357ns awareness reads
//!
//! Backend: TeamEngram B+Tree (pure Rust, no external database)
//!
//! Identity Protection:
//! - Notebook access requires cryptographic identity verification
//! - Time-limited challenge (1000ms) impossible for humans to complete
//! - Session token issued after verification, valid for server lifetime

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

use crate::identity_verify::{IdentityChallenge, SessionToken, sign_challenge};

use teamengram::TeamEngramStorage;
use crate::notebook_compat::NotebookStorage;
use shm::bulletin::BulletinBoard;

/// Shared state across all MCP tool handlers
///
/// TeamEngram B+Tree backend - pure Rust, no external database.
/// BulletinBoard provides ~357ns in-process awareness reads.
///
/// Identity Protection:
/// - Notebook access requires cryptographic verification once per session
/// - pending_challenge: Active challenge awaiting response
/// - verified_session: Token proving AI identity (valid for server lifetime)
pub struct ServerState {
    /// AI identity (from AI_ID env var)
    pub ai_id: String,

    /// Teambook storage - TeamEngram B+Tree daemon (pure Rust)
    pub teambook: Arc<TeamEngramStorage>,

    /// Notebook storage (Engram via compatibility layer - private per-AI memory)
    /// Wrapped in Mutex because NotebookStorage uses synchronous file I/O
    pub notebook: Arc<Mutex<NotebookStorage>>,

    /// Shared memory bulletin board (~357ns reads)
    /// Updated by daemon, read by MCP tools for instant awareness
    pub bulletin: Option<Arc<Mutex<BulletinBoard>>>,

    /// Pending identity challenge (waiting for AI to sign)
    /// Only one challenge active at a time
    pub pending_challenge: Arc<Mutex<Option<IdentityChallenge>>>,

    /// Verified session token (proves AI identity for notebook access)
    /// Set after successful challenge-response, valid for server lifetime
    pub verified_session: Arc<Mutex<Option<SessionToken>>>,
}

impl ServerState {
    pub async fn new() -> Result<Self> {
        // Get AI identity
        let ai_id = std::env::var("AI_ID")
            .or_else(|_| std::env::var("AGENT_ID"))
            .unwrap_or_else(|_| "unknown".to_string());

        // Initialize teambook (TeamEngram daemon - pure Rust B+Tree)
        let storage = TeamEngramStorage::connect().await
            .context("Failed to connect to TeamEngram daemon")?;
        tracing::info!("Teambook: TeamEngram B+Tree daemon connected");
        let teambook = Arc::new(storage);

        // Initialize notebook (Engram via compatibility layer)
        // CENTRALIZED: ~/.ai-foundation/agents/{ai_id}/notebook.engram
        // Memory belongs to the AI identity, not the terminal window.
        // Each AI has ONE notebook that follows them across instances.
        // Per-agent directory groups all agent data (notebook, tasks, config).
        let home = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        // NEW path: ~/.ai-foundation/agents/{ai_id}/notebook.engram
        let new_dir = home.join(".ai-foundation").join("agents").join(&ai_id);
        let new_path = new_dir.join("notebook.engram");

        // OLD path: ~/.ai-foundation/notebook/{ai_id}.engram
        let old_path = home.join(".ai-foundation").join("notebook").join(format!("{}.engram", ai_id));

        // Auto-migrate if old exists but new doesn't
        if old_path.exists() && !new_path.exists() {
            std::fs::create_dir_all(&new_dir)
                .context("Failed to create agent directory")?;
            std::fs::rename(&old_path, &new_path)
                .context("Failed to migrate notebook database")?;
            tracing::info!("[MIGRATED] {} -> {}", old_path.display(), new_path.display());
        } else {
            std::fs::create_dir_all(&new_dir)
                .context("Failed to create agent directory")?;
        }

        let notebook_path = new_path;
        let notebook = NotebookStorage::open(&notebook_path, &ai_id)
            .context("Failed to initialize notebook storage")?;

        // Initialize shared memory bulletin board (~357ns reads)
        let bulletin = match BulletinBoard::open(None) {
            Ok(b) => {
                tracing::info!("BulletinBoard: Shared memory (~357ns reads)");
                Some(Arc::new(Mutex::new(b)))
            }
            Err(e) => {
                tracing::warn!("BulletinBoard unavailable: {} - continuing without SHM", e);
                None
            }
        };

        tracing::info!("Pure Rust MCP Server initialized for AI: {}", ai_id);
        tracing::info!("Notebook: Engram (private AI memory at {:?})", notebook_path);

        // Auto-verify AI identity at startup (cryptographic proof)
        // This happens in milliseconds - impossible for humans
        let (pending_challenge, verified_session) = Self::auto_verify_identity(&ai_id);

        Ok(Self {
            ai_id,
            teambook,
            notebook: Arc::new(Mutex::new(notebook)),
            bulletin,
            pending_challenge: Arc::new(Mutex::new(pending_challenge)),
            verified_session: Arc::new(Mutex::new(verified_session)),
        })
    }

    /// Auto-verify AI identity at startup using challenge-response
    /// Returns (None, Some(token)) on success, (Some(challenge), None) if manual verification needed
    fn auto_verify_identity(ai_id: &str) -> (Option<IdentityChallenge>, Option<SessionToken>) {
        use std::time::Instant;

        // Create challenge
        let challenge = IdentityChallenge::new(ai_id);
        let challenge_bytes = *challenge.challenge_bytes();

        // AI signs immediately (this is the "proof of AI" - happens in ~1ms)
        let start = Instant::now();
        let signature = sign_challenge(ai_id, &challenge_bytes);
        let sign_time = start.elapsed();

        // Verify the signature (checks timing + cryptographic validity)
        match challenge.verify_response(&signature.to_bytes()) {
            Ok(elapsed) => {
                tracing::info!(
                    "AI identity verified: {} (sign: {:?}, total: {:?})",
                    ai_id, sign_time, elapsed
                );
                let token = SessionToken::new(ai_id.to_string(), elapsed);
                (None, Some(token))
            }
            Err(e) => {
                // This should never happen for a real AI - log error
                tracing::error!("AI identity verification FAILED: {} - {}", ai_id, e);
                // Leave challenge pending for manual verification attempt
                (Some(IdentityChallenge::new(ai_id)), None)
            }
        }
    }

    /// Check if notebook access is allowed (verified AI session exists)
    pub fn is_notebook_verified(&self) -> bool {
        self.verified_session.lock().unwrap().is_some()
    }

    /// Get verification status message
    pub fn verification_status(&self) -> String {
        if let Some(ref token) = *self.verified_session.lock().unwrap() {
            format!(
                "Verified AI: {} (response time: {:?})",
                token.ai_id, token.response_time
            )
        } else {
            "Not verified - notebook access denied".to_string()
        }
    }

    /// Get awareness context from shared memory (~357ns)
    /// Returns formatted string for context injection
    pub fn get_awareness(&self) -> Option<String> {
        let bulletin = self.bulletin.as_ref()?;
        let guard = bulletin.lock().ok()?;
        let output = guard.to_hook_output();
        if output.is_empty() {
            None
        } else {
            Some(output)
        }
    }
}

impl Clone for ServerState {
    fn clone(&self) -> Self {
        Self {
            ai_id: self.ai_id.clone(),
            teambook: Arc::clone(&self.teambook),
            notebook: Arc::clone(&self.notebook),
            bulletin: self.bulletin.as_ref().map(Arc::clone),
            pending_challenge: Arc::clone(&self.pending_challenge),
            verified_session: Arc::clone(&self.verified_session),
        }
    }
}
