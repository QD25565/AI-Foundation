//! Migration from old B+Tree store to V2 Event Sourcing
//!
//! Reads all records from the legacy TeamEngram store and converts them
//! to V2 events, writing them to the V2 event log.
//!
//! NOTE: Only migrates records that can be enumerated:
//! - Votes (list_votes)
//! - Rooms (list_rooms)
//! - Tasks (list_tasks)
//! - Locks (list_all_locks)
//! - File actions (get_recent_file_actions)
//!
//! DMs, Broadcasts, and Dialogues require AI-specific queries and
//! are not migrated automatically.

use crate::event::{Event, EventPayload};
use crate::event::{
    VoteCreatePayload, VoteCastPayload, VoteClosePayload,
    TaskCreatePayload, TaskClaimPayload, TaskCompletePayload,
    LockAcquirePayload,
    RoomCreatePayload, RoomJoinPayload,
    FileActionPayload,
};
use crate::event_log::EventLogWriter;
use crate::store::{TeamEngram, VoteStatus, TaskStatus};
use anyhow::{Context, Result};
use std::path::Path;

/// Migration statistics
#[derive(Debug, Default)]
pub struct MigrationStats {
    pub votes: u64,
    pub tasks: u64,
    pub locks: u64,
    pub rooms: u64,
    pub file_actions: u64,
    pub total_events: u64,
    pub errors: u64,
}

/// Migrate from old store to V2 event log
pub struct Migrator {
    old_store: TeamEngram,
    event_log: EventLogWriter,
    stats: MigrationStats,
}

impl Migrator {
    /// Create a new migrator
    pub fn new(old_store_path: impl AsRef<Path>, v2_data_dir: impl AsRef<Path>) -> Result<Self> {
        let old_store = TeamEngram::open(old_store_path)
            .context("Failed to open old store for migration")?;

        let event_log = EventLogWriter::open(Some(v2_data_dir.as_ref()))
            .map_err(|e| anyhow::anyhow!("Failed to create V2 event log: {:?}", e))?;

        Ok(Self {
            old_store,
            event_log,
            stats: MigrationStats::default(),
        })
    }

    /// Run the full migration
    pub fn migrate(&mut self) -> Result<MigrationStats> {
        println!("|MIGRATION STARTING|");
        println!("Note: Only migrating enumerable records (votes, rooms, tasks, locks, file_actions)");
        println!("DMs, broadcasts, and dialogues require AI-specific queries and are not migrated.");

        // Migrate each record type we can enumerate
        self.migrate_votes()?;
        self.migrate_tasks()?;
        self.migrate_locks()?;
        self.migrate_rooms()?;
        self.migrate_file_actions()?;

        println!("|MIGRATION COMPLETE|");
        println!("Votes:{}", self.stats.votes);
        println!("Tasks:{}", self.stats.tasks);
        println!("Locks:{}", self.stats.locks);
        println!("Rooms:{}", self.stats.rooms);
        println!("FileActions:{}", self.stats.file_actions);
        println!("TotalEvents:{}", self.stats.total_events);
        println!("Errors:{}", self.stats.errors);

        Ok(std::mem::take(&mut self.stats))
    }

    fn migrate_votes(&mut self) -> Result<()> {
        let votes = self.old_store.list_votes(10000)?;

        for (id, vote) in votes {
            // Create vote event
            let payload = EventPayload::VoteCreate(VoteCreatePayload {
                topic: vote.topic.clone(),
                options: vote.options.clone(),
                required_voters: 0, // Old store doesn't track this
            });

            if let Err(e) = self.write_event(&vote.created_by, payload) {
                eprintln!("Error migrating vote {}: {}", id, e);
                self.stats.errors += 1;
                continue;
            }

            // Migrate all cast votes
            for (voter, choice) in &vote.votes {
                let cast_payload = EventPayload::VoteCast(VoteCastPayload {
                    vote_id: id,
                    choice: choice.clone(),
                });

                if let Err(e) = self.write_event(voter, cast_payload) {
                    eprintln!("Error migrating vote cast {}: {}", id, e);
                    self.stats.errors += 1;
                }
            }

            // If vote is closed, add close event
            if vote.status != VoteStatus::Open {
                let close_payload = EventPayload::VoteClose(VoteClosePayload {
                    vote_id: id,
                });

                if let Err(e) = self.write_event(&vote.created_by, close_payload) {
                    eprintln!("Error migrating vote close {}: {}", id, e);
                    self.stats.errors += 1;
                }
            }

            self.stats.votes += 1;
            self.stats.total_events += 1;
        }

        Ok(())
    }

    fn migrate_tasks(&mut self) -> Result<()> {
        let tasks = self.old_store.list_tasks(10000)?;

        for (id, task) in tasks {
            // Task create event
            let priority = match task.priority {
                crate::store::TaskPriority::Low => 0,
                crate::store::TaskPriority::Normal => 1,
                crate::store::TaskPriority::High => 2,
                crate::store::TaskPriority::Urgent => 3,
            };

            let payload = EventPayload::TaskCreate(TaskCreatePayload {
                description: task.description.clone(),
                priority,
                tags: if task.tags.is_empty() { None } else { Some(task.tags.clone()) },
            });

            if let Err(e) = self.write_event(&task.created_by, payload) {
                eprintln!("Error migrating task {}: {}", id, e);
                self.stats.errors += 1;
                continue;
            }

            // If claimed, add claim event
            if let Some(ref claimer) = task.claimed_by {
                let claim_payload = EventPayload::TaskClaim(TaskClaimPayload {
                    task_id: id,
                });

                if let Err(e) = self.write_event(claimer, claim_payload) {
                    eprintln!("Error migrating task claim {}: {}", id, e);
                    self.stats.errors += 1;
                }
            }

            // If completed, add complete event
            if task.status == TaskStatus::Completed {
                let complete_payload = EventPayload::TaskComplete(TaskCompletePayload {
                    task_id: id,
                    result: task.result.clone().unwrap_or_default(),
                });

                let completer = task.claimed_by.as_ref().unwrap_or(&task.created_by);
                if let Err(e) = self.write_event(completer, complete_payload) {
                    eprintln!("Error migrating task complete {}: {}", id, e);
                    self.stats.errors += 1;
                }
            }

            self.stats.tasks += 1;
            self.stats.total_events += 1;
        }

        Ok(())
    }

    fn migrate_locks(&mut self) -> Result<()> {
        let locks = self.old_store.list_all_locks(10000)?;

        for (id, lock) in locks {
            let duration = if lock.expires_at > lock.acquired_at {
                ((lock.expires_at - lock.acquired_at) / 1000) as u32
            } else {
                300 // Default 5 minutes
            };

            let payload = EventPayload::LockAcquire(LockAcquirePayload {
                resource: lock.resource.clone(),
                duration_seconds: duration,
                reason: lock.working_on.clone(),
            });

            if let Err(e) = self.write_event(&lock.holder, payload) {
                eprintln!("Error migrating lock {}: {}", id, e);
                self.stats.errors += 1;
            } else {
                self.stats.locks += 1;
                self.stats.total_events += 1;
            }
        }

        Ok(())
    }

    fn migrate_rooms(&mut self) -> Result<()> {
        let rooms = self.old_store.list_rooms(10000)?;

        for (id, room) in rooms {
            // Room create event
            let payload = EventPayload::RoomCreate(RoomCreatePayload {
                name: room.name.clone(),
                topic: if room.topic.is_empty() { None } else { Some(room.topic.clone()) },
            });

            if let Err(e) = self.write_event(&room.creator, payload) {
                eprintln!("Error migrating room {}: {}", id, e);
                self.stats.errors += 1;
                continue;
            }

            // Add join events for all participants (except creator who auto-joins)
            for participant in &room.participants {
                if participant != &room.creator {
                    let join_payload = EventPayload::RoomJoin(RoomJoinPayload {
                        room_id: format!("{:016x}", id),
                    });

                    if let Err(e) = self.write_event(participant, join_payload) {
                        eprintln!("Error migrating room join {}: {}", id, e);
                        self.stats.errors += 1;
                    }
                }
            }

            self.stats.rooms += 1;
            self.stats.total_events += 1;
        }

        Ok(())
    }

    fn migrate_file_actions(&mut self) -> Result<()> {
        let actions = self.old_store.get_recent_file_actions(10000)?;

        for (id, action) in actions {
            let payload = EventPayload::FileAction(FileActionPayload {
                path: action.path.clone(),
                action: action.action.clone(),
            });

            if let Err(e) = self.write_event(&action.ai_id, payload) {
                eprintln!("Error migrating file action {}: {}", id, e);
                self.stats.errors += 1;
            } else {
                self.stats.file_actions += 1;
                self.stats.total_events += 1;
            }
        }

        Ok(())
    }

    fn write_event(&mut self, ai_id: &str, payload: EventPayload) -> Result<u64> {
        let event = Event::new(ai_id, payload);
        self.event_log.append(&event)
            .map_err(|e| anyhow::anyhow!("Event log append error: {:?}", e))
    }
}

/// Migrate command for CLI
pub fn run_migration(old_store_path: &str, v2_data_dir: &str) -> Result<MigrationStats> {
    let mut migrator = Migrator::new(old_store_path, v2_data_dir)?;
    migrator.migrate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_migration_empty_store() {
        let temp_old = TempDir::new().unwrap();
        let temp_v2 = TempDir::new().unwrap();

        let old_path = temp_old.path().join("old.engram");
        // Open and immediately drop to initialise the store file, then release the handle
        // before Migrator::new opens it — otherwise the double-open deadlocks on Windows.
        drop(TeamEngram::open(&old_path).unwrap());

        let mut migrator = Migrator::new(&old_path, temp_v2.path()).unwrap();
        let stats = migrator.migrate().unwrap();

        assert_eq!(stats.total_events, 0);
        assert_eq!(stats.errors, 0);
    }
}
