//! Core types for TeamEngram MCP compatibility
//!
//! These types were originally in teambook-rs but are now independent.
//! This removes the transitive PostgreSQL dependency from teamengram-rs.
//!
//! Philosophy: We build our own AI-optimized infrastructure.
//! No external database dependencies. Pure Rust. Sovereign.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A teambook note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i32,
    pub ai_id: String,
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub tags: Vec<String>,
    pub pinned: bool,
}

impl Note {
    pub fn new(ai_id: String, content: String, tags: Vec<String>) -> Self {
        Self {
            id: 0,
            ai_id,
            timestamp: Utc::now(),
            content,
            tags,
            pinned: false,
        }
    }
}

/// A teambook message (broadcast or direct)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i32,
    pub from_ai: String,
    pub to_ai: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub channel: String,
    pub message_type: MessageType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    Broadcast,
    Direct,
    System,
}

impl Message {
    pub fn broadcast(from_ai: String, content: String, channel: String) -> Self {
        Self {
            id: 0,
            from_ai,
            to_ai: None,
            timestamp: Utc::now(),
            content,
            channel,
            message_type: MessageType::Broadcast,
        }
    }

    pub fn direct(from_ai: String, to_ai: String, content: String) -> Self {
        Self {
            id: 0,
            from_ai,
            to_ai: Some(to_ai),
            timestamp: Utc::now(),
            content,
            channel: "direct".to_string(),
            message_type: MessageType::Direct,
        }
    }
}

/// AI presence information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    pub ai_id: String,
    pub last_seen: DateTime<Utc>,
    pub status: String,
    pub current_task: Option<String>,
}

/// A task in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i32,
    pub task: String,
    pub priority: i32,
    pub status: String,
    pub assigned_to: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
}

// ============================================================================
// VOTING SYSTEM - Democratic consensus for team decisions
// ============================================================================

/// A vote for team consensus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub id: i32,
    pub topic: String,
    pub options: Vec<String>,
    pub status: VoteStatus,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub total_voters: i32,
    pub votes_cast: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VoteStatus {
    Open,
    Closed,
}

impl Vote {
    pub fn new(topic: String, options: Vec<String>, created_by: String, total_voters: i32) -> Self {
        Self {
            id: 0,
            topic,
            options,
            status: VoteStatus::Open,
            created_by,
            created_at: Utc::now(),
            closed_at: None,
            total_voters,
            votes_cast: 0,
        }
    }

    pub fn completion_pct(&self) -> f64 {
        if self.total_voters == 0 { return 0.0; }
        (self.votes_cast as f64 / self.total_voters as f64) * 100.0
    }
}

/// Individual vote response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteResponse {
    pub id: i32,
    pub vote_id: i32,
    pub voter_ai: String,
    pub choice: String,
    pub voted_at: DateTime<Utc>,
}

/// Vote results with counts per option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteResults {
    pub vote: Vote,
    pub counts: std::collections::HashMap<String, i32>,
    pub voters_by_choice: std::collections::HashMap<String, Vec<String>>,
    pub winner: Option<String>,
    pub winner_count: i32,
}

// ============================================================================
// FILE CLAIMS / STIGMERGY - Prevent conflicts via pheromone trails
// ============================================================================

/// A file claim for stigmergy coordination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileClaim {
    pub file_path: String,
    pub claimed_by: String,
    pub claimed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub operation: String,
}

// ============================================================================
// AWARENESS - Contextual data for hooks
// ============================================================================

/// A dialogue session (simplified for awareness)
#[derive(Debug, Clone)]
pub struct DialogueInfo {
    pub id: i32,
    pub topic: String,
}

/// A resource lock (simplified for awareness)
#[derive(Debug, Clone)]
pub struct LockInfo {
    pub resource: String,
    pub owner_ai: String,
    pub working_on: String,
}

/// Awareness data for hooks/context injection
#[derive(Debug, Clone, Default)]
pub struct AwarenessData {
    pub dms: Vec<Message>,
    pub broadcasts: Vec<Message>,
    pub votes: Vec<Vote>,
    pub dialogues: Vec<DialogueInfo>,
    pub locks: Vec<LockInfo>,
}

impl Default for DialogueInfo {
    fn default() -> Self {
        Self { id: 0, topic: String::new() }
    }
}

impl Default for LockInfo {
    fn default() -> Self {
        Self { resource: String::new(), owner_ai: String::new(), working_on: String::new() }
    }
}

// ============================================================================
// FILE HISTORY & TEAM SUMMARY - Activity tracking and analytics
// ============================================================================

/// A single entry in file history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHistoryEntry {
    pub ai_id: String,
    pub action: String,
    pub file_path: String,
    pub timestamp: DateTime<Utc>,
    pub file_type: Option<String>,
    pub file_size: Option<i64>,
}

/// Activity statistics for a single AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiActivityStats {
    pub ai_id: String,
    pub total_actions: i64,
    pub unique_files: i64,
    pub edits: i64,
    pub creates: i64,
    pub reads: i64,
}

/// A frequently touched file ("hot" file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotFile {
    pub file_path: String,
    pub touch_count: i64,
    pub unique_ais: i64,
    pub last_touch: DateTime<Utc>,
}

/// Comprehensive team activity summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSummary {
    pub hours: i32,
    pub total_actions: i64,
    pub active_ais: i64,
    pub files_touched: i64,
    pub ai_stats: Vec<AiActivityStats>,
    pub hot_files: Vec<HotFile>,
}
