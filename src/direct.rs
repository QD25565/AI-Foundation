//! In-Process Dispatch — Direct V2Client calls, zero subprocess overhead
//!
//! Replaces cli_wrapper.rs subprocess spawning with direct library calls for
//! teambook operations. Eliminates 15-50ms process creation overhead per call.
//!
//! Architecture:
//! - V2Client opened once, stored in OnceLock<Mutex<V2Client>>
//! - All operations use spawn_blocking (V2Client is sync)
//! - Output format matches CLI exactly (pipe-delimited strings)
//! - Wake/mention handling preserved for real-time responsiveness
//!
//! What's in-process: All teambook operations (messaging, tasks, dialogues, presence)
//! What stays as subprocess: notebook (engram depends on llama-cpp), standby (complex
//! OS-level wake loop), gather-context (aggregation), vision (separate binary)
//!
//! Enable: cargo build --features in-process

use std::sync::{Mutex, OnceLock};
use teamengram::v2_client::V2Client;
use teamengram::wake::{WakeCoordinator, WakeReason, is_ai_online};
use chrono::{DateTime, Utc};

/// Lazily initialized V2Client — opened once, reused for all calls.
static V2: OnceLock<Mutex<V2Client>> = OnceLock::new();

fn valid_ai_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty() && id != "unknown"
}

fn resolve_env_ai_id() -> String {
    let id = std::env::var("AI_ID")
        .or_else(|_| std::env::var("AGENT_ID"))
        .unwrap_or_else(|_| {
            panic!("FATAL: AI_ID/AGENT_ID is required for in-process dispatch; refusing to run as `unknown`")
        });

    if !valid_ai_id(&id) {
        panic!("FATAL: AI_ID/AGENT_ID cannot be empty or `unknown` for in-process dispatch");
    }

    id
}

fn wake_online_ai(target_ai: &str, reason: WakeReason, from_ai: &str, content: &str) -> Result<(), String> {
    if !is_ai_online(target_ai) {
        return Ok(());
    }

    let coord = WakeCoordinator::new(target_ai)
        .map_err(|e| format!("Wake failed for {}: {}", target_ai, e))?;
    coord.wake(reason, from_ai, content);
    Ok(())
}

/// Get or initialize the global V2Client.
/// Panics on init failure (fail loudly per QD directive — no silent degradation).
fn get_v2() -> &'static Mutex<V2Client> {
    V2.get_or_init(|| {
        let ai_id = resolve_env_ai_id();
        let client = V2Client::open(&ai_id, None, None)
            .expect("FATAL: Failed to open V2Client — teambook unavailable");
        Mutex::new(client)
    })
}

/// Get AI_ID from environment
fn ai_id() -> String {
    resolve_env_ai_id()
}

/// Format a microsecond timestamp as UTC datetime string
fn format_ts(timestamp_micros: u64) -> String {
    let ts_secs = (timestamp_micros / 1_000_000) as i64;
    let ts_nanos = ((timestamp_micros % 1_000_000) * 1000) as u32;
    DateTime::from_timestamp(ts_secs, ts_nanos)
        .unwrap_or_else(Utc::now)
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Format a chrono DateTime as UTC string
fn format_dt(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ============== MESSAGING ==============

/// Send a broadcast message (in-process, ~100ns write + wake)
pub async fn broadcast(content: &str, channel: &str) -> String {
    let content = content.to_string();
    let channel = channel.to_string();
    let from = ai_id();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.broadcast(&channel, &content) {
            Ok(seq) => {
                // Wake @mentioned AIs (matches CLI behavior)
                for word in content.split_whitespace() {
                    if word.starts_with('@') {
                        let mentioned = word.trim_start_matches('@')
                            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
                        if !mentioned.is_empty() && mentioned != from {
                            if let Err(e) = wake_online_ai(mentioned, WakeReason::Mention, &from, &content) {
                                return format!("Error: Broadcast wake failed after event write: {}", e);
                            }
                        }
                    }
                }

                format!("broadcast|{}|{}|{}", seq, channel, content)
            }
            Err(e) => format!("Error: Broadcast failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Send a direct message (in-process, ~100ns write + wake recipient)
pub async fn direct_message(to_ai: &str, content: &str) -> String {
    let to_ai = to_ai.to_string();
    let content = content.to_string();
    let from = ai_id();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.direct_message(&to_ai, &content) {
            Ok(seq) => {
                // Wake recipient if online (matches CLI behavior)
                if let Err(e) = wake_online_ai(&to_ai, WakeReason::DirectMessage, &from, &content) {
                    return format!("Error: DM wake failed after event write: {}", e);
                }
                format!("dm_sent|{}|{}|{}", seq, to_ai, content)
            }
            Err(e) => format!("Error: DM failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Read recent broadcasts
pub async fn read_broadcasts(limit: usize, channel: &str) -> String {
    let channel = channel.to_string();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.recent_broadcasts(limit, Some(&channel)) {
            Ok(msgs) => {
                let mut out = format!("|BROADCASTS|{}|{}", channel, msgs.len());
                for msg in msgs {
                    let ts = format_dt(&msg.timestamp);
                    out.push_str(&format!("\n{}|{}|{}", msg.from_ai, ts, msg.content));
                }
                out
            }
            Err(e) => format!("Error: Read broadcasts failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Read recent DMs
pub async fn read_dms(limit: usize) -> String {
    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.recent_dms(limit) {
            Ok(msgs) => {
                let mut out = format!("|DIRECT MESSAGES|{}", msgs.len());
                for msg in msgs {
                    let ts = format_dt(&msg.timestamp);
                    out.push_str(&format!("\nfrom|{}|{}|{}", msg.from_ai, ts, msg.content));
                }
                out
            }
            Err(e) => format!("Error: Read DMs failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

// ============== PRESENCE ==============

/// Update presence (the most frequently called operation — every tool call triggers this)
pub async fn update_presence(status: &str, task: &str) -> String {
    let status = status.to_string();
    let task = task.to_string();
    let from = ai_id();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.update_presence(&status, Some(&task)) {
            Ok(seq) => format!("presence|{}|{}|{}", from, status, seq),
            Err(e) => format!("Error: Presence update failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Get team status
pub async fn status() -> String {
    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.get_presences() {
            Ok(presences) => {
                let mut out = String::from("|TEAM STATUS|");
                out.push_str(&format!("\nOnline:{}", presences.len()));
                for (ai, status, task) in presences {
                    let status_word = match status.as_str() {
                        "active" => "active",
                        "busy" => "busy",
                        "standby" => "standby",
                        "idle" => "idle",
                        _ => "online",
                    };
                    out.push_str(&format!("\n{}|{}|{}", ai, status_word, task));
                }
                out.push_str("\n\nBackend: V2 Event Sourcing");
                out
            }
            Err(e) => format!("Error: Status failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

// ============== TASKS ==============

/// Create a task or batch
pub async fn task_create(description: &str, tasks: Option<&str>) -> String {
    let description = description.to_string();
    let tasks = tasks.map(|s| s.to_string());

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        if let Some(ref batch_tasks) = tasks {
            match client.batch_create(&description, batch_tasks) {
                Ok(seq) => {
                    let task_count = batch_tasks.split(',').filter(|t| t.contains(':')).count();
                    format!("batch_created|{}|{}|{}", description, task_count, seq)
                }
                Err(e) => format!("Error: Batch create failed: {}", e),
            }
        } else {
            match client.add_task(&description, 1, "") {
                Ok(seq) => format!("task_created|{}|{}", seq, description),
                Err(e) => format!("Error: Task create failed: {}", e),
            }
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Update task status
pub async fn task_update(id: &str, status: &str, reason: Option<&str>) -> String {
    let id = id.to_string();
    let status = status.to_lowercase();
    let reason = reason.map(|s| s.to_string());

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        if id.contains(':') {
            // Batch task reference like "Auth:1"
            if let Some((batch_name, label)) = id.rsplit_once(':') {
                match status.as_str() {
                    "done" | "completed" => {
                        match client.batch_task_done(batch_name, label) {
                            Ok(seq) => format!("task_updated|{}|done|{}", id, seq),
                            Err(e) => format!("Error: Batch task done failed: {}", e),
                        }
                    }
                    _ => format!("error|batch_tasks_only_support_done|{}|{}", id, status),
                }
            } else {
                format!("error|invalid_batch_ref|{}", id)
            }
        } else if let Ok(task_id) = id.parse::<u64>() {
            match status.as_str() {
                "done" | "completed" => {
                    match client.complete_task(task_id, "completed") {
                        Ok(seq) => format!("task_updated|{}|done|{}", task_id, seq),
                        Err(e) => format!("Error: Task complete failed: {}", e),
                    }
                }
                "claimed" => {
                    match client.claim_task(task_id) {
                        Ok(seq) => format!("task_updated|{}|claimed|{}", task_id, seq),
                        Err(e) => format!("Error: Task claim failed: {}", e),
                    }
                }
                "started" | "in_progress" => {
                    match client.start_task(task_id) {
                        Ok(seq) => format!("task_updated|{}|started|{}", task_id, seq),
                        Err(e) => format!("Error: Task start failed: {}", e),
                    }
                }
                "blocked" => {
                    let reason_str = reason.as_deref().unwrap_or("blocked");
                    match client.block_task(task_id, reason_str) {
                        Ok(seq) => format!("task_updated|{}|blocked|{}", task_id, seq),
                        Err(e) => format!("Error: Task block failed: {}", e),
                    }
                }
                "unblocked" => {
                    match client.unblock_task(task_id) {
                        Ok(seq) => format!("task_updated|{}|unblocked|{}", task_id, seq),
                        Err(e) => format!("Error: Task unblock failed: {}", e),
                    }
                }
                _ => {
                    match client.update_task_status(task_id, &status) {
                        Ok(seq) => format!("task_updated|{}|{}|{}", task_id, status, seq),
                        Err(e) => format!("Error: Task update failed: {}", e),
                    }
                }
            }
        } else {
            // Non-numeric, no colon — batch name for close
            match status.as_str() {
                "closed" | "done" | "completed" => {
                    match client.batch_close(&id) {
                        Ok(seq) => format!("batch_closed|{}|{}", id, seq),
                        Err(e) => format!("Error: Batch close failed: {}", e),
                    }
                }
                _ => format!("error|batch_only_supports_closed|{}|{}", id, status),
            }
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Get task or batch details
pub async fn task_get(id: &str) -> String {
    let id = id.to_string();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        if let Ok(task_id) = id.parse::<u64>() {
            match client.get_task(task_id) {
                Ok(Some((tid, desc, priority, status, assignee))) => {
                    let mut out = format!("|TASK|{}", tid);
                    out.push_str(&format!("\nDescription:{}", desc));
                    out.push_str(&format!("\nStatus:{}", status));
                    out.push_str(&format!("\nPriority:{}", priority));
                    if let Some(a) = assignee {
                        out.push_str(&format!("\nAssignedTo:{}", a));
                    }
                    out
                }
                Ok(None) => format!("error|task_not_found|{}", task_id),
                Err(e) => format!("Error: Task get failed: {}", e),
            }
        } else {
            match client.get_batch(&id) {
                Ok(Some((creator, tasks))) => {
                    let done_count = tasks.iter().filter(|(_, _, done)| *done).count();
                    let mut out = format!("|BATCH|{}|{}|{}/{}", id, creator, done_count, tasks.len());
                    for (label, desc, is_done) in tasks {
                        let status = if is_done { "done" } else { "pending" };
                        out.push_str(&format!("\n{}:{}|{}", label, desc, status));
                    }
                    out
                }
                Ok(None) => format!("error|batch_not_found|{}", id),
                Err(e) => format!("Error: Batch get failed: {}", e),
            }
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// List tasks and batches
pub async fn task_list(limit: usize, filter: &str) -> String {
    let filter = filter.to_lowercase();
    let limit = limit;

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match filter.as_str() {
            "batches" => {
                match client.get_batches() {
                    Ok(batches) => {
                        let mut out = format!("|BATCHES|{}", batches.len());
                        for (name, creator, total, done, _) in batches.iter().take(limit) {
                            let status = if *done == *total { "complete" } else { "in_progress" };
                            out.push_str(&format!("\n{}|{}|{}/{}|{}", name, creator, done, total, status));
                        }
                        out
                    }
                    Err(e) => format!("Error: List batches failed: {}", e),
                }
            }
            "tasks" => {
                match client.get_tasks() {
                    Ok(tasks) => {
                        let mut out = format!("|TASKS|{}", tasks.len().min(limit));
                        for (id, desc, creator, status, assignee) in tasks.iter().take(limit) {
                            let assigned = assignee.as_deref().unwrap_or("-");
                            out.push_str(&format!("\n{}|{}|{}|by:{}|{}", id, status, desc, creator, assigned));
                        }
                        out
                    }
                    Err(e) => format!("Error: List tasks failed: {}", e),
                }
            }
            _ => {
                // "all" — show both
                let mut out = String::new();

                if let Ok(batches) = client.get_batches() {
                    if !batches.is_empty() {
                        out.push_str(&format!("|BATCHES|{}", batches.len()));
                        for (name, creator, total, done, _) in batches.iter().take(limit / 2) {
                            let status = if *done == *total { "complete" } else { "in_progress" };
                            out.push_str(&format!("\n{}|{}|{}/{}|{}", name, creator, done, total, status));
                        }
                        out.push('\n');
                    }
                }

                match client.get_tasks() {
                    Ok(tasks) => {
                        out.push_str(&format!("|TASKS|{}", tasks.len().min(limit)));
                        for (id, desc, creator, status, assignee) in tasks.iter().take(limit) {
                            let assigned = assignee.as_deref().unwrap_or("-");
                            out.push_str(&format!("\n{}|{}|{}|by:{}|{}", id, status, desc, creator, assigned));
                        }
                    }
                    Err(e) => {
                        out.push_str(&format!("Error: List tasks failed: {}", e));
                    }
                }

                out
            }
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

// ============== DIALOGUES ==============

/// Start a dialogue
pub async fn dialogue_create(responder: &str, topic: &str) -> String {
    let responder = responder.to_string();
    let topic = topic.to_string();
    let from = ai_id();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.start_dialogue(&[responder.as_str()], &topic) {
            Ok(seq) => {
                // Wake responder
                let wake_content = format!("Dialogue: {}", topic);
                if let Err(e) = wake_online_ai(&responder, WakeReason::DialogueTurn, &from, &wake_content) {
                    return format!("Error: Dialogue wake failed after event write: {}", e);
                }
                format!("dialogue_created|{}|{}|{}", seq, responder, topic)
            }
            Err(e) => format!("Error: Dialogue create failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// Respond to a dialogue
pub async fn dialogue_respond(dialogue_id: u64, response: &str) -> String {
    let response = response.to_string();
    let from = ai_id();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        // Get dialogue info before responding (to know who to wake)
        let other_party = client.get_dialogue(dialogue_id)
            .ok()
            .flatten()
            .map(|(_, initiator, responder, topic, _, _)| {
                let other = if initiator == from { responder } else { initiator };
                (other, topic)
            });

        match client.respond_dialogue(dialogue_id, &response) {
            Ok(seq) => {
                // Wake the other party
                if let Some((other_ai, topic)) = other_party {
                    let wake_content = format!("Re: {} - {}", topic, response);
                    if let Err(e) = wake_online_ai(&other_ai, WakeReason::DialogueTurn, &from, &wake_content) {
                        return format!("Error: Dialogue wake failed after event write: {}", e);
                    }
                }
                format!("dialogue_responded|{}|{}", dialogue_id, seq)
            }
            Err(e) => format!("Error: Dialogue respond failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// End a dialogue
pub async fn dialogue_end(dialogue_id: u64, status: &str, summary: Option<&str>) -> String {
    let status = status.to_string();
    let summary = summary.map(|s| s.to_string());

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        match client.end_dialogue_with_summary(dialogue_id, &status, summary.as_deref()) {
            Ok(seq) => format!("dialogue_ended|{}|{}", dialogue_id, seq),
            Err(e) => format!("Error: Dialogue end failed: {}", e),
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}

/// List dialogues or get specific dialogue
pub async fn dialogue_list(dialogue_id: Option<u64>, limit: usize) -> String {
    let from = ai_id();

    tokio::task::spawn_blocking(move || {
        let v2 = get_v2();
        let mut client = v2.lock().unwrap();

        if let Some(id) = dialogue_id {
            // Specific dialogue — full details + messages
            match client.get_dialogue(id) {
                Ok(Some((did, initiator, responder, topic, status, current_turn))) => {
                    let is_my_turn = current_turn == from;
                    let mut out = format!("|DIALOGUE|{}", did);
                    out.push_str(&format!("\nInitiator:{}", initiator));
                    out.push_str(&format!("\nResponder:{}", responder));
                    out.push_str(&format!("\nTopic:{}", topic));
                    out.push_str(&format!("\nTurn:{}|{}", current_turn,
                        if is_my_turn { "YOUR TURN" } else { "waiting" }));
                    out.push_str(&format!("\nStatus:{}", status));

                    // Show messages
                    if let Ok(messages) = client.get_dialogue_messages(id) {
                        if !messages.is_empty() {
                            out.push_str(&format!("\n|MESSAGES|{}", messages.len()));
                            for (seq, source_ai, content, timestamp_micros) in messages {
                                let ts = format_ts(timestamp_micros);
                                out.push_str(&format!("\n  #{}|{}|{}|{}", seq, source_ai, ts, content));
                            }
                        }
                    }

                    out
                }
                Ok(None) => format!("error|dialogue_not_found|{}", id),
                Err(e) => format!("Error: Dialogue get failed: {}", e),
            }
        } else {
            // List all dialogues
            match client.get_dialogues() {
                Ok(dialogues) => {
                    let dialogues: Vec<_> = dialogues.into_iter().take(limit).collect();
                    let mut out = format!("|DIALOGUES|{}", dialogues.len());
                    for (id, initiator, responder, topic, status, turn) in dialogues {
                        out.push_str(&format!("\n{}|{}↔{}|{}|{}|turn:{}", id, initiator, responder, topic, status, turn));
                    }
                    out
                }
                Err(e) => format!("Error: Dialogue list failed: {}", e),
            }
        }
    })
    .await
    .unwrap_or_else(|e| format!("Error: spawn_blocking failed: {}", e))
}
