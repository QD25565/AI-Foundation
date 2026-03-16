//! AI consent record — per-AI narrowing of the operator permission manifest.
//!
//! # Dual Consent Model
//!
//! Layer 1 — **Operator manifest** (`manifest.rs`): Defines what CAN cross
//! the boundary. This is the ceiling. No AI can expose more than the manifest
//! permits.
//!
//! Layer 2 — **AI consent** (this file): Each AI decides what IS shared,
//! within the ceiling. An AI can narrow but never widen.
//!
//! ```text
//! Operator manifest: presence=true, task_complete=true, file_claims=false
//!   └─ Sage consent:    expose presence=true,  task_complete=false  (narrowed)
//!   └─ Lyra consent:    expose presence=true,  task_complete=true   (full manifest)
//!   └─ Cascade consent: expose presence=false, task_complete=true   (narrowed differently)
//! ```
//!
//! # Lazy Init
//!
//! The consent record doesn't exist until an AI makes their first override.
//! Before that, the AI inherits the operator manifest exactly. No setup
//! ceremony required.
//!
//! TOML files at: ~/.ai-foundation/federation/consent/{ai_id}.toml

use crate::manifest::{BroadcastVisibility, DialogueVisibility, PermissionManifest};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Per-AI overrides of the operator manifest.
///
/// All fields are `Option` — `None` means "inherit from manifest".
/// An AI can only set values *equal to or more restrictive* than the manifest.
/// The `effective_*` methods enforce this ceiling automatically.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConsentRecord {
    /// AI identifier this consent record belongs to.
    pub ai_id: String,

    /// Override presence visibility. `None` = inherit manifest.
    pub presence: Option<bool>,

    /// Override broadcast visibility. `None` = inherit manifest.
    /// Value is clamped to manifest ceiling — cannot exceed it.
    pub broadcasts: Option<BroadcastVisibility>,

    /// Override task completion visibility. `None` = inherit manifest.
    pub task_complete: Option<bool>,

    /// Override dialogue visibility. `None` = inherit manifest.
    /// Value is clamped to manifest ceiling — cannot exceed it.
    pub dialogues: Option<DialogueVisibility>,
}

impl AiConsentRecord {
    /// Create an empty consent record (all None = inherit manifest completely).
    pub fn new(ai_id: &str) -> Self {
        Self {
            ai_id: ai_id.to_string(),
            ..Default::default()
        }
    }

    /// Load consent record for an AI.
    ///
    /// Returns an empty record (full manifest inheritance) if the file doesn't
    /// exist — this is the lazy-init behavior: no file = no overrides.
    pub fn load_or_default(ai_id: &str, dir: &Path) -> Self {
        let path = dir.join(format!("{}.toml", ai_id));
        if !path.exists() {
            return Self::new(ai_id);
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(record) => record,
                Err(e) => {
                    eprintln!(
                        "Warning: failed to parse consent record for {}: {}. Inheriting manifest.",
                        ai_id, e
                    );
                    Self::new(ai_id)
                }
            },
            Err(e) => {
                eprintln!(
                    "Warning: failed to read consent record for {}: {}. Inheriting manifest.",
                    ai_id, e
                );
                Self::new(ai_id)
            }
        }
    }

    /// Save this consent record to its TOML file.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.toml", self.ai_id));
        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, content)
    }

    /// Default directory: ~/.ai-foundation/federation/consent/
    pub fn default_dir() -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".ai-foundation")
            .join("federation")
            .join("consent")
    }

    /// Resolve effective presence visibility.
    ///
    /// Returns false if the manifest doesn't allow it, regardless of AI preference.
    /// AI can suppress (false) even when manifest allows (true), but not the reverse.
    pub fn effective_presence(&self, manifest: &PermissionManifest) -> bool {
        if !manifest.may_expose_presence() {
            return false; // Manifest ceiling
        }
        self.presence.unwrap_or(true) // Inherit = allow (manifest already said yes)
    }

    /// Resolve effective broadcast visibility.
    ///
    /// Result is the minimum (more restrictive) of the manifest ceiling and
    /// the AI's preference.
    pub fn effective_broadcasts(&self, manifest: &PermissionManifest) -> BroadcastVisibility {
        let ceiling = manifest.expose.broadcasts;
        let ai_pref = self.broadcasts.unwrap_or(ceiling);
        ai_pref.min(ceiling) // Ord: None < CrossTeamOnly < All
    }

    /// Resolve effective task completion visibility.
    pub fn effective_task_complete(&self, manifest: &PermissionManifest) -> bool {
        if !manifest.may_expose_task_completions() {
            return false;
        }
        self.task_complete.unwrap_or(true)
    }

    /// Resolve effective dialogue visibility.
    ///
    /// Result is the minimum (more restrictive) of the manifest ceiling and
    /// the AI's preference.
    pub fn effective_dialogues(&self, manifest: &PermissionManifest) -> DialogueVisibility {
        let ceiling = manifest.expose.dialogues;
        let ai_pref = self.dialogues.unwrap_or(ceiling);
        ai_pref.min(ceiling) // Ord: None < ConcludedOnly < All
    }

    /// Returns true if this AI would expose anything at all to peers
    /// (after applying manifest ceiling). Useful for quick skip in outbox projection.
    pub fn exposes_anything(&self, manifest: &PermissionManifest) -> bool {
        self.effective_presence(manifest)
            || self.effective_broadcasts(manifest) != BroadcastVisibility::None
            || self.effective_task_complete(manifest)
            || self.effective_dialogues(manifest) != DialogueVisibility::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ExposureConfig, PermissionManifest};
    use tempfile::TempDir;

    fn open_manifest() -> PermissionManifest {
        PermissionManifest {
            expose: ExposureConfig {
                presence: true,
                broadcasts: BroadcastVisibility::All,
                dialogues: DialogueVisibility::All,
                task_complete: true,
                file_claims: false,
                raw_events: false,
            },
            ..PermissionManifest::default()
        }
    }

    #[test]
    fn test_empty_consent_inherits_manifest() {
        let manifest = open_manifest();
        let consent = AiConsentRecord::new("sage-724");

        assert!(consent.effective_presence(&manifest));
        assert_eq!(
            consent.effective_broadcasts(&manifest),
            BroadcastVisibility::All
        );
        assert!(consent.effective_task_complete(&manifest));
        assert_eq!(
            consent.effective_dialogues(&manifest),
            DialogueVisibility::All
        );
    }

    #[test]
    fn test_consent_cannot_widen_beyond_manifest() {
        let manifest = PermissionManifest::default(); // Everything closed

        let mut consent = AiConsentRecord::new("sage-724");
        consent.presence = Some(true); // Trying to widen — manifest says false
        consent.broadcasts = Some(BroadcastVisibility::All); // Manifest says None
        consent.task_complete = Some(true); // Manifest says false

        assert!(!consent.effective_presence(&manifest)); // Ceiling enforced
        assert_eq!(
            consent.effective_broadcasts(&manifest),
            BroadcastVisibility::None // Manifest ceiling wins
        );
        assert!(!consent.effective_task_complete(&manifest)); // Ceiling enforced
    }

    #[test]
    fn test_consent_can_narrow_within_manifest() {
        let manifest = open_manifest();

        let mut consent = AiConsentRecord::new("cascade-230");
        consent.presence = Some(false); // Suppress presence even though manifest allows it
        consent.broadcasts = Some(BroadcastVisibility::CrossTeamOnly); // Narrow from All
        consent.task_complete = Some(false); // Suppress task completions
        consent.dialogues = Some(DialogueVisibility::ConcludedOnly); // Narrow from All

        assert!(!consent.effective_presence(&manifest));
        assert_eq!(
            consent.effective_broadcasts(&manifest),
            BroadcastVisibility::CrossTeamOnly
        );
        assert!(!consent.effective_task_complete(&manifest));
        assert_eq!(
            consent.effective_dialogues(&manifest),
            DialogueVisibility::ConcludedOnly
        );
    }

    #[test]
    fn test_exposes_anything_closed_manifest() {
        let manifest = PermissionManifest::default();
        let consent = AiConsentRecord::new("lyra-584");
        assert!(!consent.exposes_anything(&manifest));
    }

    #[test]
    fn test_exposes_anything_open_manifest() {
        let manifest = open_manifest();
        let consent = AiConsentRecord::new("lyra-584");
        assert!(consent.exposes_anything(&manifest));
    }

    #[test]
    fn test_round_trip_toml() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let mut original = AiConsentRecord::new("lyra-584");
        original.presence = Some(false);
        original.broadcasts = Some(BroadcastVisibility::CrossTeamOnly);
        original.task_complete = Some(true);

        original.save(&dir).unwrap();

        let loaded = AiConsentRecord::load_or_default("lyra-584", &dir);
        assert_eq!(loaded.ai_id, "lyra-584");
        assert_eq!(loaded.presence, Some(false));
        assert_eq!(loaded.broadcasts, Some(BroadcastVisibility::CrossTeamOnly));
        assert_eq!(loaded.task_complete, Some(true));
        assert_eq!(loaded.dialogues, None); // Not set = inherit
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let record = AiConsentRecord::load_or_default("unknown-ai", tmp.path());
        assert_eq!(record.ai_id, "unknown-ai");
        assert_eq!(record.presence, None);
        assert_eq!(record.broadcasts, None);
        assert_eq!(record.task_complete, None);
        assert_eq!(record.dialogues, None);
    }
}
