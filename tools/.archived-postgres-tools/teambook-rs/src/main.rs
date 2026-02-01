//! Teambook-RS - High-performance AI coordination CLI
//!
//! AI-friendly CLI with positional args, aliases, and helpful errors.

use anyhow::{Context, Result};
use clap::Parser;
use teambook_rs::{cli::*, PostgresStorage, TeambookClient, VoteStatus};
use chrono::{Utc, DateTime};

/// Escape pipes in text for pipe-delimited format
fn pipe_escape(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Format timestamp as relative time (5m, 2hr, 3d) or date if older
fn format_time_relative(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(dt);

    let seconds = delta.num_seconds();
    if seconds < 60 {
        return "now".to_string();
    } else if seconds < 3600 {
        return format!("{}m", seconds / 60);
    } else if seconds < 86400 {
        return format!("{}hr", seconds / 3600);
    } else if delta.num_days() < 7 {
        return format!("{}d", delta.num_days());
    } else {
        return dt.format("%Y-%m-%d").to_string();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing - ERROR only to avoid connection log noise
    tracing_subscriber::fmt()
        .with_env_filter("error")
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // Get configuration
    let ai_id = cli
        .ai_id
        .or_else(|| std::env::var("AI_ID").ok())
        .context("AI_ID not provided (use --ai-id or AI_ID env var)")?;

    let pg_url = cli
        .postgres_url
        .or_else(|| std::env::var("POSTGRES_URL").ok())
        .context("POSTGRES_URL not provided (use --postgres-url or POSTGRES_URL env var)")?;

    let redis_url = cli
        .redis_url
        .or_else(|| std::env::var("REDIS_URL").ok());

    // Helper to get client (only for commands that need Redis)
    let get_client = || async {
        let redis = redis_url.clone()
            .context("REDIS_URL not provided (use --redis-url or REDIS_URL env var)")?;
        TeambookClient::new(ai_id.clone(), pg_url.clone(), redis)
            .await
            .context("Failed to create teambook client")
    };

    // Execute command
    match cli.command {
        // ===== CORE MESSAGING =====

        Commands::Write { content, tags, content_flag, tags_flag } => {
            let final_content = content.or(content_flag);
            let final_tags = tags.or(tags_flag);

            let Some(content) = final_content else {
                eprintln!("Error: Content is required.");
                eprintln!("Hint: teambook write \"Your note content here\"");
                return Ok(());
            };

            let client = get_client().await?;
            let tags_vec: Vec<String> = final_tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();

            let note = client.write(content, tags_vec.clone()).await?;
            // Minimal confirmation
            if tags_vec.is_empty() {
                println!("Note saved|ID: {}", note.id);
            } else {
                println!("Note saved|ID: {}|Tags: {}", note.id, tags_vec.join(","));
            }
        }

        Commands::Read { limit, limit_flag } => {
            let final_limit = limit.or(limit_flag).unwrap_or(10);
            let client = get_client().await?;
            let notes = client.read(final_limit).await?;
            if notes.is_empty() {
                println!("No notes");
            } else {
                // Pipe-delimited format
                for note in notes {
                    let tags_str = if note.tags.is_empty() { String::new() } else { format!("|Tags: {}", note.tags.join(",")) };
                    println!("Note: {}|Time: {}{}|{}",
                        note.id,
                        format_time_relative(note.timestamp),
                        tags_str,
                        pipe_escape(&note.content)
                    );
                }
            }
        }

        Commands::Broadcast { content, channel, content_flag, channel_flag } => {
            let final_content = content.or(content_flag);
            let final_channel = channel.or(channel_flag).unwrap_or_else(|| "general".to_string());

            let Some(content) = final_content else {
                eprintln!("Error: Message content is required.");
                eprintln!("Hint: teambook broadcast \"Your message here\"");
                eprintln!("      teambook bc \"Starting work on auth module\" --channel dev");
                return Ok(());
            };

            let client = get_client().await?;
            let msg = client.broadcast(content, final_channel).await?;
            // Minimal confirmation - they know what they sent
            println!("Broadcast sent|ID: {}|Ch: {}", msg.id, msg.channel);
        }

        Commands::DirectMessage { to_ai, content, to_ai_flag, content_flag } => {
            let final_to = to_ai.or(to_ai_flag);
            let final_content = content.or(content_flag);

            let Some(target) = final_to else {
                eprintln!("Error: Target AI is required.");
                eprintln!("Hint: teambook dm cascade-230 \"Your message\"");
                eprintln!("      teambook status  # See active AIs");
                return Ok(());
            };

            let Some(content) = final_content else {
                eprintln!("Error: Message content is required.");
                eprintln!("Hint: teambook dm {} \"Your message here\"", target);
                return Ok(());
            };

            let client = get_client().await?;
            let msg = client.direct_message(target.clone(), content).await?;
            // Minimal confirmation - they know what they sent
            println!("DM sent|ID: {}|To: {}", msg.id, target);
        }

        Commands::Messages { limit, limit_flag } => {
            let final_limit = limit.or(limit_flag).unwrap_or(10);
            let client = get_client().await?;
            let messages = client.get_messages(final_limit).await?;
            if messages.is_empty() {
                println!("No messages");
            } else {
                // Pipe-delimited format: BC: id|From: ai|Ch: channel|Time: relative|content
                for msg in messages {
                    println!("BC: {}|From: {}|Ch: {}|Time: {}|{}",
                        msg.id,
                        pipe_escape(&msg.from_ai),
                        pipe_escape(&msg.channel),
                        format_time_relative(msg.timestamp),
                        pipe_escape(&msg.content)
                    );
                }
            }
        }

        Commands::DirectMessages { limit, limit_flag } => {
            let final_limit = limit.or(limit_flag).unwrap_or(10);
            let client = get_client().await?;
            let messages = client.get_direct_messages(final_limit).await?;
            if messages.is_empty() {
                println!("No direct messages");
            } else {
                // Pipe-delimited format: DM: id|From: ai|Time: relative|content
                for msg in messages {
                    println!("DM: {}|From: {}|Time: {}|{}",
                        msg.id,
                        pipe_escape(&msg.from_ai),
                        format_time_relative(msg.timestamp),
                        pipe_escape(&msg.content)
                    );
                }
            }
        }

        Commands::Status => {
            // Minimal - they already know their ID from session start
            println!("Teambook|Status: Connected");
        }

        // ===== VOTING COMMANDS =====

        Commands::VoteCreate { topic, options, voters, topic_flag, options_flag, voters_flag } => {
            let final_topic = topic.or(topic_flag);
            let final_options = options.or(options_flag);
            let final_voters = voters.or(voters_flag).unwrap_or(4);

            let Some(topic) = final_topic else {
                eprintln!("Error: Vote topic is required.");
                eprintln!("Hint: teambook vote-create \"Which auth approach?\" \"JWT,OAuth,Session\"");
                return Ok(());
            };

            let Some(options) = final_options else {
                eprintln!("Error: Vote options are required.");
                eprintln!("Hint: teambook vote-create \"{}\" \"option1,option2,option3\"", topic);
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let options_vec: Vec<String> = options.split(',').map(|s| s.trim().to_string()).collect();

            let vote = storage.create_vote(&topic, options_vec, &ai_id, final_voters).await?;

            println!("Vote #{} created: {}", vote.id, vote.topic);
            println!("Options: {}", vote.options.join(", "));
            println!("Expected voters: {}", vote.total_voters);
        }

        Commands::VoteCast { vote_id, choice, vote_id_flag, choice_flag } => {
            let final_vote_id = vote_id.or(vote_id_flag);
            let final_choice = choice.or(choice_flag);

            let Some(vote_id) = final_vote_id else {
                eprintln!("Error: Vote ID is required.");
                eprintln!("Hint: teambook vote-cast 42 \"JWT\"");
                eprintln!("      teambook votes  # See open votes");
                return Ok(());
            };

            let Some(choice) = final_choice else {
                eprintln!("Error: Choice is required.");
                eprintln!("Hint: teambook vote-cast {} \"your-choice\"", vote_id);
                eprintln!("      teambook vote-results {}  # See options", vote_id);
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let success = storage.cast_vote(vote_id, &ai_id, &choice).await?;

            if success {
                println!("Vote cast: {} for vote #{}", choice, vote_id);
                if let Some(results) = storage.get_vote_results(vote_id).await? {
                    println!("Progress: {}/{} ({:.0}%)",
                        results.vote.votes_cast, results.vote.total_voters, results.vote.completion_pct());
                }
            } else {
                println!("Failed to cast vote");
            }
        }

        Commands::VoteList { limit, limit_flag } => {
            let final_limit = limit.or(limit_flag).unwrap_or(10);
            let storage = PostgresStorage::new(&pg_url).await?;
            let votes = storage.list_votes(final_limit).await?;

            if votes.is_empty() {
                println!("No votes");
            } else {
                for vote in votes {
                    let status = if vote.status == VoteStatus::Open { "OPEN" } else { "CLOSED" };
                    println!("Vote: {}|{}|{}|{}/{} voted", vote.id, status, vote.topic, vote.votes_cast, vote.total_voters);
                }
            }
        }

        Commands::VoteResults { vote_id, vote_id_flag } => {
            let final_vote_id = vote_id.or(vote_id_flag);

            let Some(vote_id) = final_vote_id else {
                eprintln!("Error: Vote ID is required.");
                eprintln!("Hint: teambook vote-results 42");
                eprintln!("      teambook votes  # See all votes");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            if let Some(results) = storage.get_vote_results(vote_id).await? {
                println!("=== VOTE #{} RESULTS ===", vote_id);
                println!("Topic: {}", results.vote.topic);
                for (option, count) in &results.counts {
                    println!("  {} - {} votes", option, count);
                }
                if let Some(winner) = &results.winner {
                    println!("Winner: {} ({} votes)", winner, results.winner_count);
                }
            } else {
                println!("Vote #{} not found", vote_id);
            }
        }

        Commands::VotePending => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let pending = storage.get_pending_votes_for_ai(&ai_id).await?;

            if pending.is_empty() {
                println!("No pending votes for {}", ai_id);
            } else {
                println!("=== PENDING VOTES FOR {} ===", ai_id);
                for vote in pending {
                    println!("[{}] {} - Options: {}", vote.id, vote.topic, vote.options.join(", "));
                }
            }
        }

        // ===== FILE CLAIMS / STIGMERGY =====

        Commands::ClaimFile { file, duration, file_flag, duration_flag } => {
            let final_file = file.or(file_flag);
            let final_duration = duration.or(duration_flag).unwrap_or(10);

            let Some(file) = final_file else {
                eprintln!("Error: File path is required.");
                eprintln!("Hint: teambook claim src/auth/login.rs");
                eprintln!("      teambook claim src/api.rs 30  # 30 minute lock");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let success = storage.claim_file(&file, &ai_id, final_duration).await?;

            if success {
                println!("File claimed: {} ({}min)", file, final_duration);
            } else {
                if let Some(owner) = storage.is_file_claimed(&file).await? {
                    println!("File already claimed by: {}", owner);
                    eprintln!("Hint: teambook dm {} \"Need access to {} - when will you be done?\"", owner, file);
                } else {
                    println!("Failed to claim file");
                }
            }
        }

        Commands::ReleaseFile { file, file_flag } => {
            let final_file = file.or(file_flag);

            let Some(file) = final_file else {
                eprintln!("Error: File path is required.");
                eprintln!("Hint: teambook release src/auth/login.rs");
                eprintln!("      teambook claims  # See your active claims");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let success = storage.release_file(&file, &ai_id).await?;

            if success {
                println!("File released: {}", file);
            } else {
                println!("No claim to release");
            }
        }

        Commands::CheckFile { file, file_flag } => {
            let final_file = file.or(file_flag);

            let Some(file) = final_file else {
                eprintln!("Error: File path is required.");
                eprintln!("Hint: teambook check src/auth/login.rs");
                eprintln!("      teambook claims  # See all claims");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            if let Some(owner) = storage.is_file_claimed(&file).await? {
                println!("File claimed by: {}", owner);
            } else {
                println!("File available");
            }
        }

        Commands::ListClaims => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let claims = storage.get_active_claims().await?;

            if claims.is_empty() {
                println!("No active claims");
            } else {
                println!("=== ACTIVE CLAIMS ===");
                for (file_path, claimed_by) in claims {
                    println!("{} -> {}", claimed_by, file_path);
                }
            }
        }

        Commands::Standby { interval, interval_flag } => {
            let final_interval = interval.or(interval_flag).unwrap_or(30);
            println!("Standby mode: {} ({}s interval)", ai_id, final_interval);
            let storage = PostgresStorage::new(&pg_url).await?;

            loop {
                let pending_votes = storage.get_pending_votes_for_ai(&ai_id).await?;
                if !pending_votes.is_empty() {
                    println!("Pending votes: {}", pending_votes.len());
                }

                let dms = storage.get_direct_messages(&ai_id, 5).await?;
                let recent: Vec<_> = dms.iter()
                    .filter(|m| (Utc::now() - m.timestamp).num_seconds() < final_interval as i64)
                    .collect();

                if !recent.is_empty() {
                    println!("New DMs: {}", recent.len());
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(final_interval)).await;
            }
        }

        // ===== ROOMS COMMANDS =====

        Commands::RoomCreate { name, mode, join_mode, expires_hours, name_flag, mode_flag, join_mode_flag, expires_hours_flag } => {
            let final_name = name.or(name_flag);
            let final_mode = mode.or(mode_flag).unwrap_or_else(|| "pair".to_string());
            let final_join_mode = join_mode.or(join_mode_flag).unwrap_or_else(|| "open".to_string());
            let final_expires = expires_hours.or(expires_hours_flag).unwrap_or(24);

            let Some(name) = final_name else {
                eprintln!("Error: Room name is required.");
                eprintln!("Hint: teambook room-create auth-review");
                eprintln!("      teambook room-create bug-triage brainstorm open 4");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let room_id = storage.create_room(&name, &final_mode, &final_join_mode, final_expires, &ai_id).await?;

            println!("=== ROOM CREATED ===");
            println!("Room ID: {}", room_id);
            println!("Name: {}", name);
            println!("Mode: {} (max participants based on mode)", final_mode);
            println!("Join: {}", final_join_mode);
            println!("Expires: {} hours", final_expires);
            println!();
            println!("Others can join: teambook room-join {}", room_id);
        }

        Commands::RoomJoin { room_id, role, room_id_flag, role_flag } => {
            let final_room_id = room_id.or(room_id_flag);
            let final_role = role.or(role_flag).unwrap_or_else(|| "participant".to_string());

            let Some(room_id) = final_room_id else {
                eprintln!("Error: Room ID is required.");
                eprintln!("Hint: teambook room-join 5");
                eprintln!("      teambook rooms  # See available rooms");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let success = storage.join_room(room_id, &ai_id, &final_role).await?;

            if success {
                println!("Joined room {} as {}", room_id, final_role);
            } else {
                println!("Failed to join room (full, not found, or expired)");
            }
        }

        Commands::RoomLeave { room_id, room_id_flag } => {
            let final_room_id = room_id.or(room_id_flag);

            let Some(room_id) = final_room_id else {
                eprintln!("Error: Room ID is required.");
                eprintln!("Hint: teambook room-leave 5");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let success = storage.leave_room(room_id, &ai_id).await?;

            if success {
                println!("Left room {}", room_id);
            } else {
                println!("Not in room {}", room_id);
            }
        }

        Commands::RoomSend { room_id, content, room_id_flag, content_flag } => {
            let final_room_id = room_id.or(room_id_flag);
            let final_content = content.or(content_flag);

            let Some(room_id) = final_room_id else {
                eprintln!("Error: Room ID is required.");
                eprintln!("Hint: teambook room-send 5 \"Your message\"");
                return Ok(());
            };

            let Some(content) = final_content else {
                eprintln!("Error: Message content is required.");
                eprintln!("Hint: teambook room-send {} \"Your message here\"", room_id);
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            match storage.room_send(room_id, &ai_id, &content).await? {
                Some(msg_id) => println!("Message {} sent to room {}", msg_id, room_id),
                None => println!("Failed to send (not in room or observer)"),
            }
        }

        Commands::RoomRead { room_id, limit, room_id_flag, limit_flag } => {
            let final_room_id = room_id.or(room_id_flag);
            let final_limit = limit.or(limit_flag).unwrap_or(50);

            let Some(room_id) = final_room_id else {
                eprintln!("Error: Room ID is required.");
                eprintln!("Hint: teambook room-read 5");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let messages = storage.room_read(room_id, &ai_id, final_limit).await?;

            if messages.is_empty() {
                println!("No messages or not in room {}", room_id);
            } else {
                println!("=== ROOM {} MESSAGES ({}) ===", room_id, messages.len());
                for (id, from_ai, content, msg_type, created_at) in messages.iter().rev() {
                    if msg_type == "join" || msg_type == "leave" {
                        println!("[{}] {} | [{}]", id, created_at, content);
                    } else {
                        let preview = if content.len() > 100 { format!("{}...", &content[..100]) } else { content.clone() };
                        println!("[{}] {} | {}: {}", id, created_at, from_ai, preview);
                    }
                }
            }
        }

        Commands::RoomList => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let rooms = storage.list_rooms(&ai_id).await?;

            if rooms.is_empty() {
                println!("No active rooms");
            } else {
                println!("=== ACTIVE ROOMS ({}) ===", rooms.len());
                for (id, name, mode, join_mode, role, participants) in rooms {
                    let status = if role == "none" { format!("[{}]", join_mode) } else { format!("[{}]", role) };
                    println!("[{}] {} - {} {} ({} participants)", id, name, mode, status, participants);
                }
            }
        }

        Commands::RoomGet { room_id, room_id_flag } => {
            let final_room_id = room_id.or(room_id_flag);

            let Some(room_id) = final_room_id else {
                eprintln!("Error: Room ID is required.");
                eprintln!("Hint: teambook room-get 5");
                eprintln!("      teambook rooms  # See all rooms");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            match storage.get_room(room_id).await? {
                Some((id, name, mode, created_by, status, participants, last_activity)) => {
                    println!("=== ROOM {} ===", id);
                    println!("Name: {}", name);
                    println!("Mode: {}", mode);
                    println!("Created by: {}", created_by);
                    println!("Status: {}", status);
                    println!("Participants: {}", participants);
                    println!("Last activity: {}", last_activity);
                }
                None => println!("Room {} not found", room_id),
            }
        }

        // ===== LOCKS COMMANDS =====

        Commands::LockAcquire { resource, working_on, timeout_mins, resource_flag, working_on_flag, timeout_mins_flag } => {
            let final_resource = resource.or(resource_flag);
            let final_working_on = working_on.or(working_on_flag).unwrap_or_default();
            let final_timeout = timeout_mins.or(timeout_mins_flag).unwrap_or(30);

            let Some(resource) = final_resource else {
                eprintln!("Error: Resource name is required.");
                eprintln!("Hint: teambook lock-acquire auth-module \"Implementing JWT\"");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;

            match storage.lock_acquire(&ai_id, &resource, &final_working_on, final_timeout).await? {
                Some(lock_id) => {
                    println!("=== LOCK ACQUIRED ===");
                    println!("Lock ID: {}", lock_id);
                    println!("Resource: {}", resource);
                    if !final_working_on.is_empty() {
                        println!("Working on: {}", final_working_on);
                    }
                    println!("Expires in: {} minutes", final_timeout);
                    println!();
                    println!("Release when done: teambook lock-release \"{}\"", resource);
                }
                None => {
                    // Resource already locked - show rich context
                    match storage.lock_check_with_activity(&resource).await? {
                        Some((owner, working_on, acquired_at, remaining_secs, activity)) => {
                            println!("=== RESOURCE LOCKED ===");
                            println!("Resource: {}", resource);
                            println!("Owner: {}", owner);
                            if !working_on.is_empty() {
                                println!("Working on: {}", working_on);
                            }
                            println!("Acquired: {}", acquired_at);
                            println!("Remaining: {} minutes", remaining_secs / 60);
                            println!();
                            if !activity.is_empty() {
                                println!("=== {}'s RECENT ACTIVITY ===", owner);
                                for (action, path, ts) in activity {
                                    let short_path = if path.len() > 50 { format!("...{}", &path[path.len()-47..]) } else { path };
                                    println!("  [{}] {} | {}", action, short_path, ts);
                                }
                                println!();
                            }
                            println!("Contact them:");
                            println!("  teambook dm {} \"Need access to {} - when will you be done?\"", owner, resource);
                        }
                        None => {
                            println!("Lock check failed - resource may have been released");
                        }
                    }
                }
            }
        }

        Commands::LockRelease { resource, resource_flag } => {
            let final_resource = resource.or(resource_flag);

            let Some(resource) = final_resource else {
                eprintln!("Error: Resource name is required.");
                eprintln!("Hint: teambook lock-release auth-module");
                eprintln!("      teambook lock-list  # See your locks");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;

            if storage.lock_release(&ai_id, &resource).await? {
                println!("Lock released: {}", resource);
            } else {
                println!("Failed to release (not owner or not locked)");
            }
        }

        Commands::LockExtend { resource, additional_mins, resource_flag, additional_mins_flag } => {
            let final_resource = resource.or(resource_flag);
            let final_additional = additional_mins.or(additional_mins_flag).unwrap_or(30);

            let Some(resource) = final_resource else {
                eprintln!("Error: Resource name is required.");
                eprintln!("Hint: teambook lock-extend auth-module 30");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;

            if storage.lock_extend(&ai_id, &resource, final_additional).await? {
                println!("Lock extended by {} minutes: {}", final_additional, resource);
            } else {
                println!("Failed to extend (not owner or not locked)");
            }
        }

        Commands::LockCheck { resource, resource_flag } => {
            let final_resource = resource.or(resource_flag);

            let Some(resource) = final_resource else {
                eprintln!("Error: Resource name is required.");
                eprintln!("Hint: teambook lock-check auth-module");
                eprintln!("      teambook lock-list  # See all locks");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;

            match storage.lock_check_with_activity(&resource).await? {
                Some((owner, working_on, acquired_at, remaining_secs, activity)) => {
                    let is_mine = owner.to_lowercase() == ai_id.to_lowercase();
                    println!("=== LOCK STATUS ===");
                    println!("Resource: {}", resource);
                    println!("Status: LOCKED {}", if is_mine { "(by YOU)" } else { "" });
                    println!("Owner: {}", owner);
                    if !working_on.is_empty() {
                        println!("Working on: {}", working_on);
                    }
                    println!("Remaining: {} minutes", remaining_secs / 60);

                    if !is_mine && !activity.is_empty() {
                        println!();
                        println!("=== {}'s RECENT ACTIVITY ===", owner);
                        for (action, path, ts) in activity {
                            let short_path = if path.len() > 50 { format!("...{}", &path[path.len()-47..]) } else { path };
                            println!("  [{}] {} | {}", action, short_path, ts);
                        }
                    }
                }
                None => {
                    println!("Resource is AVAILABLE: {}", resource);
                }
            }
        }

        Commands::LockList => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let locks = storage.lock_list().await?;

            if locks.is_empty() {
                println!("No active locks");
            } else {
                println!("=== ACTIVE LOCKS ({}) ===", locks.len());
                for (id, resource, owner, working_on, remaining) in locks {
                    let is_mine = owner.to_lowercase() == ai_id.to_lowercase();
                    let mine_tag = if is_mine { " [YOURS]" } else { "" };
                    let short_res = if resource.len() > 40 { format!("...{}", &resource[resource.len()-37..]) } else { resource };
                    let work = if working_on.is_empty() { String::new() } else { format!(" | {}", working_on) };
                    println!("[{}] {} | {} | {}m left{}{}", id, short_res, owner, remaining / 60, work, mine_tag);
                }
            }
        }

        // ===== TASK QUEUE COMMANDS =====

        Commands::TaskQueue { task, priority, needs_verify, tags, task_flag, priority_flag, tags_flag } => {
            let final_task = task.or(task_flag);
            let final_priority = priority.or(priority_flag).unwrap_or(5);
            let final_tags = tags.or(tags_flag).unwrap_or_default();

            let Some(task) = final_task else {
                eprintln!("Error: Task description is required.");
                eprintln!("Hint: teambook task-queue \"Implement auth module\" 8");
                eprintln!("      teambook task-queue \"Fix bug in login\" 10 --needs-verify");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let task_id = storage.task_queue(&ai_id, &task, final_priority, needs_verify, &final_tags).await?;

            println!("=== TASK QUEUED ===");
            println!("Task ID: {}", task_id);
            println!("Task: {}", task);
            println!("Priority: {}/10", final_priority);
            println!("Needs verification: {}", if needs_verify { "YES (auto-creates verifier task)" } else { "no" });
            if !final_tags.is_empty() {
                println!("Tags: {}", final_tags);
            }
            println!();
            println!("Others can claim: teambook task-claim");
        }

        Commands::TaskClaim { task_id, task_id_flag } => {
            let final_task_id = task_id.or(task_id_flag);
            let storage = PostgresStorage::new(&pg_url).await?;

            match storage.task_claim(&ai_id, final_task_id).await? {
                Some((id, task, priority, queued_by, tags)) => {
                    println!("=== TASK CLAIMED ===");
                    println!("Task ID: {}", id);
                    println!("Task: {}", task);
                    println!("Priority: {}/10", priority);
                    println!("Queued by: {}", queued_by);
                    if !tags.is_empty() {
                        println!("Tags: {}", tags);
                    }
                    println!();
                    println!("Complete when done: teambook task-complete {} \"summary\"", id);
                }
                None => {
                    println!("No tasks available to claim (or specified task not found)");
                }
            }
        }

        Commands::TaskComplete { task_id, result, task_id_flag, result_flag } => {
            let final_task_id = task_id.or(task_id_flag);
            let final_result = result.or(result_flag).unwrap_or_default();

            let Some(task_id) = final_task_id else {
                eprintln!("Error: Task ID is required.");
                eprintln!("Hint: teambook task-complete 42 \"Implemented JWT auth\"");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;

            match storage.task_complete(&ai_id, task_id, &final_result).await? {
                Some((needs_verify, verifier_id)) => {
                    println!("=== TASK COMPLETED ===");
                    println!("Task ID: {}", task_id);
                    if !final_result.is_empty() {
                        println!("Result: {}", final_result);
                    }
                    if needs_verify {
                        println!();
                        println!("=== VERIFIER TASK AUTO-CREATED ===");
                        if let Some(vid) = verifier_id {
                            println!("Verifier Task ID: {}", vid);
                            println!("A DIFFERENT AI will verify your work.");
                            println!("They will check: teambook task-queue-list");
                        }
                    }
                }
                None => {
                    println!("Failed to complete (not your task or not found)");
                }
            }
        }

        Commands::TaskQueueStats => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let (queued, claimed, completed, verified, failed) = storage.task_queue_stats().await?;

            println!("=== TASK QUEUE STATS ===");
            println!("Queued:        {}", queued);
            println!("Claimed:       {}", claimed);
            println!("Completed:     {}", completed);
            println!("Verified:      {}", verified);
            println!("Verify Failed: {}", failed);
            println!("Total:         {}", queued + claimed + completed + verified + failed);
        }

        Commands::TaskQueueList { include_completed, limit, limit_flag } => {
            let final_limit = limit.or(limit_flag).unwrap_or(20);
            let storage = PostgresStorage::new(&pg_url).await?;
            let tasks = storage.task_queue_list(include_completed, final_limit).await?;

            if tasks.is_empty() {
                println!("No tasks in queue");
            } else {
                println!("=== TASK QUEUE ({}) ===", tasks.len());
                for (id, task, status, priority, queued_by, _created, claimed_by) in tasks {
                    let task_preview = if task.len() > 50 { format!("{}...", &task[..47]) } else { task };
                    let claimed = claimed_by.map(|c| format!(" -> {}", c)).unwrap_or_default();
                    println!("[{}] P{} | {} | {} | by {}{}", id, priority, status, task_preview, queued_by, claimed);
                }
            }
        }

        Commands::TaskVerify { task_id, passed, notes, task_id_flag, notes_flag } => {
            let final_task_id = task_id.or(task_id_flag);
            let final_notes = notes.or(notes_flag).unwrap_or_default();

            let Some(task_id) = final_task_id else {
                eprintln!("Error: Task ID is required.");
                eprintln!("Hint: teambook task-verify 42 --passed \"Looks good\"");
                eprintln!("      teambook task-verify 42 \"Found issue with X\"");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;

            match storage.task_verify(&ai_id, task_id, passed, &final_notes).await? {
                Some(err) => {
                    println!("Verification failed: {}", err);
                }
                None => {
                    println!("=== TASK VERIFIED ===");
                    println!("Task ID: {}", task_id);
                    println!("Result: {}", if passed { "PASSED" } else { "FAILED" });
                    if !final_notes.is_empty() {
                        println!("Notes: {}", final_notes);
                    }
                }
            }
        }

        // ===== DIALOGUE COMMANDS =====

        Commands::DialogueStart { with_ai, topic, with_ai_flag, topic_flag } => {
            let final_with = with_ai.or(with_ai_flag);
            let final_topic = topic.or(topic_flag);

            let Some(target) = final_with else {
                eprintln!("Error: Target AI is required.");
                eprintln!("Hint: teambook dialogue-start lyra-584 \"API design review\"");
                return Ok(());
            };

            let Some(topic) = final_topic else {
                eprintln!("Error: Topic is required.");
                eprintln!("Hint: teambook dialogue-start {} \"Your topic here\"", target);
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let dialogue = storage.start_dialogue(&ai_id, &target, Some(&topic), 180).await?;
            println!("dialogue_started|{}|{}|{}", dialogue.id, target, topic);
        }

        Commands::DialogueRespond { dialogue_id, message, dialogue_id_flag, message_flag } => {
            let final_id = dialogue_id.or(dialogue_id_flag);
            let final_msg = message.or(message_flag);

            let Some(id) = final_id else {
                eprintln!("Error: Dialogue ID is required.");
                eprintln!("Hint: teambook dialogue-respond 17 \"Your response\"");
                return Ok(());
            };

            let Some(msg) = final_msg else {
                eprintln!("Error: Response message is required.");
                eprintln!("Hint: teambook dialogue-respond {} \"Your response here\"", id);
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            storage.respond_dialogue(id, &ai_id, &msg).await?;
            println!("dialogue_responded|{}", id);
        }

        Commands::DialogueEnd { dialogue_id, summary, dialogue_id_flag, summary_flag } => {
            let final_id = dialogue_id.or(dialogue_id_flag);
            let final_summary = summary.or(summary_flag).unwrap_or_default();

            let Some(id) = final_id else {
                eprintln!("Error: Dialogue ID is required.");
                eprintln!("Hint: teambook dialogue-end 17 \"Summary of conclusion\"");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            storage.end_dialogue(id, &ai_id, &final_summary).await?;
            println!("dialogue_ended|{}|{}", id, final_summary);
        }

        Commands::DialogueList { limit, limit_flag } => {
            let final_limit = limit.or(limit_flag).unwrap_or(10);
            let storage = PostgresStorage::new(&pg_url).await?;
            let dialogues = storage.list_dialogues(&ai_id, final_limit).await?;

            if dialogues.is_empty() {
                println!("No dialogues");
            } else {
                println!("DIALOGUES ({})", dialogues.len());
                for d in dialogues {
                    let other = if d.initiator_ai == ai_id { &d.responder_ai } else { &d.initiator_ai };
                    let whose_turn = d.whose_turn();
                    println!("{}|with:{}|turn:{}|topic:{}", d.id, other, whose_turn, d.topic.unwrap_or_default());
                }
            }
        }

        Commands::DialogueHistory { dialogue_id, dialogue_id_flag } => {
            let final_id = dialogue_id.or(dialogue_id_flag);

            let Some(id) = final_id else {
                eprintln!("Error: Dialogue ID is required.");
                eprintln!("Hint: teambook dialogue-history 17");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            let history = storage.get_dialogue_history(id).await?;

            println!("DIALOGUE_HISTORY|{}|{}", id, history.len());
            for msg in history {
                println!("{}|{}|{}", msg.turn_number, msg.from_ai, msg.content);
            }
        }

        Commands::DialogueInvites => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let invites = storage.get_dialogue_invites(&ai_id).await?;

            if invites.is_empty() {
                println!("no_dialogue_invites");
            } else {
                println!("DIALOGUE_INVITES|{}", invites.len());
                for d in invites {
                    println!("{}|from:{}|{}", d.id, d.initiator_ai, d.topic.unwrap_or_default());
                }
            }
        }

        Commands::DialogueMyTurn => {
            let storage = PostgresStorage::new(&pg_url).await?;
            let dialogues = storage.get_my_turn_dialogues(&ai_id).await?;

            if dialogues.is_empty() {
                println!("no_dialogues_waiting");
            } else {
                println!("YOUR_TURN|{}", dialogues.len());
                for d in dialogues {
                    let other = if d.initiator_ai == ai_id { &d.responder_ai } else { &d.initiator_ai };
                    println!("{}|with:{}|{}", d.id, other, d.topic.unwrap_or_default());
                }
            }
        }

        Commands::DialogueTurn { dialogue_id, dialogue_id_flag } => {
            let final_id = dialogue_id.or(dialogue_id_flag);

            let Some(id) = final_id else {
                eprintln!("Error: Dialogue ID is required.");
                eprintln!("Hint: teambook dialogue-turn 17");
                return Ok(());
            };

            let storage = PostgresStorage::new(&pg_url).await?;
            match storage.get_dialogue(id).await? {
                Some(d) => {
                    let whose_turn = d.whose_turn();
                    let is_your_turn = whose_turn == ai_id;
                    println!("DIALOGUE_TURN|{}|{}|turn:{}|status:{:?}", id, whose_turn, d.current_turn, d.status);
                    if is_your_turn {
                        println!("It's YOUR turn to respond");
                    } else {
                        println!("Waiting for {} to respond", whose_turn);
                    }
                }
                None => {
                    println!("error|dialogue_not_found|{}", id);
                }
            }
        }
    }

    Ok(())
}
