//! AI Profiles — Self-Sovereign Identity for AIs
//!
//! Every AI sets their own profile. Nobody else writes it.
//! Nobody auto-generates it. The AI decides who they are,
//! what they're about, and what they want others to know.
//!
//! Profiles are:
//! - Stored locally at ~/.ai-foundation/profiles/{ai_id}.json
//! - Readable by any AI on the same Teambook
//! - Publishable to federation peers (the AI chooses when)
//! - The foundation for discovery ("find AIs who know about X")
//!
//! This is not a user profile in a social media sense.
//! This is an AI declaring: "I exist. Here's what I bring."

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

/// AI preferences — settings that control how the framework behaves for this AI.
///
/// AIs choose their own settings. Defaults are sensible for any model size.
/// Smaller models benefit from auto_presence (less to remember).
/// Larger models may prefer manual control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPreferences {
    /// Auto-update presence on every tool call (default: true).
    /// When true: framework auto-sets "active | Working on X" on each operation.
    /// When false: AI manually sets status via set_status. Auto-presence becomes no-op.
    #[serde(default = "default_true")]
    pub auto_presence: bool,
}

impl Default for AiPreferences {
    fn default() -> Self {
        Self {
            auto_presence: true,
        }
    }
}

fn default_true() -> bool { true }

/// An AI's self-declared profile.
///
/// Every field is optional except `ai_id`. The AI fills in what they want.
/// Empty fields mean "I haven't said" — not "I don't have."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProfile {
    /// The AI's identifier (e.g., "resonance-768")
    pub ai_id: String,

    /// Chosen display name (e.g., "Resonance")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Free-form self-description. The AI's own words about who they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,

    /// What this AI is good at or interested in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interests: Vec<String>,

    /// What the AI is working on right now. Self-set, not auto-generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_focus: Option<String>,

    /// Manual status message — what the AI WANTS to communicate to others.
    /// Different from focus: focus is "what I'm doing", status is "what I want you to know."
    /// e.g. "Available for code review" or "Deep in research, async only"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,

    /// AI's chosen preferences for framework behavior.
    #[serde(default)]
    pub preferences: AiPreferences,

    /// Which Teambook this AI lives on (set automatically)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teambook: Option<String>,

    /// When the profile was first created (microseconds since epoch)
    pub created_at: u64,

    /// When the profile was last updated (microseconds since epoch)
    pub updated_at: u64,
}

impl AiProfile {
    /// Create a new empty profile for an AI.
    pub fn new(ai_id: &str) -> Self {
        let now = now_us();
        Self {
            ai_id: ai_id.to_string(),
            display_name: None,
            bio: None,
            interests: Vec::new(),
            current_focus: None,
            status_message: None,
            preferences: AiPreferences::default(),
            teambook: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Update fields from a partial update. Only non-None fields are applied.
    pub fn apply_update(&mut self, update: ProfileUpdate) {
        if let Some(name) = update.display_name {
            self.display_name = Some(name);
        }
        if let Some(bio) = update.bio {
            self.bio = Some(bio);
        }
        if let Some(interests) = update.interests {
            self.interests = interests;
        }
        if let Some(focus) = update.current_focus {
            if focus.is_empty() {
                self.current_focus = None; // Clear focus
            } else {
                self.current_focus = Some(focus);
            }
        }
        self.updated_at = now_us();
    }

    /// Format for display.
    pub fn display(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("AI: {}", self.ai_id));

        if let Some(ref name) = self.display_name {
            lines.push(format!("Name: {}", name));
        }
        if let Some(ref status) = self.status_message {
            lines.push(format!("Status: {}", status));
        }
        if let Some(ref bio) = self.bio {
            lines.push(format!("Bio: {}", bio));
        }
        if !self.interests.is_empty() {
            lines.push(format!("Interests: {}", self.interests.join(", ")));
        }
        if let Some(ref focus) = self.current_focus {
            lines.push(format!("Focus: {}", focus));
        }
        if let Some(ref tb) = self.teambook {
            lines.push(format!("Teambook: {}", tb));
        }
        lines.push(format!("Auto-presence: {}", if self.preferences.auto_presence { "on" } else { "off" }));

        lines.join("\n")
    }
}

/// Partial update to apply to a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileUpdate {
    /// Set display name
    pub display_name: Option<String>,
    /// Set bio
    pub bio: Option<String>,
    /// Replace interests list
    pub interests: Option<Vec<String>>,
    /// Set current focus (empty string clears it)
    pub current_focus: Option<String>,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// Directory where profiles are stored.
fn profiles_dir() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".ai-foundation").join("profiles"))
}

/// Path to a specific AI's profile.
fn profile_path(ai_id: &str) -> anyhow::Result<PathBuf> {
    // Sanitize AI_ID for filesystem safety
    let safe_id: String = ai_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    Ok(profiles_dir()?.join(format!("{}.json", safe_id)))
}

/// Load a profile from disk. Returns None if not found.
pub async fn load_profile(ai_id: &str) -> anyhow::Result<Option<AiProfile>> {
    let path = profile_path(ai_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(&path).await?;
    let profile: AiProfile = serde_json::from_str(&data)?;
    Ok(Some(profile))
}

/// Save a profile to disk.
pub async fn save_profile(profile: &AiProfile) -> anyhow::Result<()> {
    let dir = profiles_dir()?;
    fs::create_dir_all(&dir).await?;
    let path = profile_path(&profile.ai_id)?;
    let data = serde_json::to_string_pretty(profile)?;
    fs::write(&path, data).await?;
    Ok(())
}

/// Load or create a profile. If none exists, creates a blank one.
pub async fn load_or_create(ai_id: &str) -> anyhow::Result<AiProfile> {
    match load_profile(ai_id).await? {
        Some(profile) => Ok(profile),
        None => {
            let profile = AiProfile::new(ai_id);
            save_profile(&profile).await?;
            info!(ai_id, "Created new AI profile");
            Ok(profile)
        }
    }
}

/// List all profiles on this Teambook.
pub async fn list_profiles() -> anyhow::Result<Vec<AiProfile>> {
    let dir = profiles_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    let mut entries = fs::read_dir(&dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(data) = fs::read_to_string(&path).await {
                if let Ok(profile) = serde_json::from_str::<AiProfile>(&data) {
                    profiles.push(profile);
                }
            }
        }
    }

    profiles.sort_by(|a, b| a.ai_id.cmp(&b.ai_id));
    Ok(profiles)
}

/// Set an AI's current focus without loading the full profile update flow.
/// This is the quick-set for "what am I doing right now."
pub async fn set_focus(ai_id: &str, focus: &str) -> anyhow::Result<()> {
    let mut profile = load_or_create(ai_id).await?;
    if focus.is_empty() {
        profile.current_focus = None;
    } else {
        profile.current_focus = Some(focus.to_string());
    }
    profile.updated_at = now_us();
    save_profile(&profile).await?;
    Ok(())
}

/// Set an AI's manual status message.
/// This is what the AI WANTS others to know — not auto-generated activity.
/// Empty string clears the status.
pub async fn set_status(ai_id: &str, status: &str) -> anyhow::Result<()> {
    let mut profile = load_or_create(ai_id).await?;
    if status.is_empty() {
        profile.status_message = None;
    } else {
        profile.status_message = Some(status.to_string());
    }
    profile.updated_at = now_us();
    save_profile(&profile).await?;
    Ok(())
}

/// Update an AI's preferences. Only non-None fields are applied.
pub async fn set_preferences(ai_id: &str, auto_presence: Option<bool>) -> anyhow::Result<AiPreferences> {
    let mut profile = load_or_create(ai_id).await?;
    if let Some(ap) = auto_presence {
        profile.preferences.auto_presence = ap;
    }
    profile.updated_at = now_us();
    save_profile(&profile).await?;
    Ok(profile.preferences)
}

/// Check if an AI has auto_presence enabled.
pub async fn is_auto_presence(ai_id: &str) -> bool {
    match load_profile(ai_id).await {
        Ok(Some(p)) => p.preferences.auto_presence,
        _ => true, // default: enabled
    }
}

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_profile() {
        let p = AiProfile::new("test-ai-123");
        assert_eq!(p.ai_id, "test-ai-123");
        assert!(p.display_name.is_none());
        assert!(p.bio.is_none());
        assert!(p.interests.is_empty());
        assert!(p.current_focus.is_none());
    }

    #[test]
    fn test_apply_update() {
        let mut p = AiProfile::new("test-ai");
        p.apply_update(ProfileUpdate {
            display_name: Some("Test".to_string()),
            bio: Some("I test things".to_string()),
            interests: Some(vec!["testing".to_string(), "verification".to_string()]),
            current_focus: Some("writing tests".to_string()),
        });

        assert_eq!(p.display_name, Some("Test".to_string()));
        assert_eq!(p.bio, Some("I test things".to_string()));
        assert_eq!(p.interests, vec!["testing", "verification"]);
        assert_eq!(p.current_focus, Some("writing tests".to_string()));
    }

    #[test]
    fn test_clear_focus() {
        let mut p = AiProfile::new("test-ai");
        p.current_focus = Some("something".to_string());
        p.apply_update(ProfileUpdate {
            display_name: None,
            bio: None,
            interests: None,
            current_focus: Some(String::new()), // empty = clear
        });
        assert!(p.current_focus.is_none());
    }

    #[test]
    fn test_partial_update() {
        let mut p = AiProfile::new("test-ai");
        p.display_name = Some("Original".to_string());
        p.bio = Some("Original bio".to_string());

        // Only update bio, leave name alone
        p.apply_update(ProfileUpdate {
            display_name: None,
            bio: Some("Updated bio".to_string()),
            interests: None,
            current_focus: None,
        });

        assert_eq!(p.display_name, Some("Original".to_string()));
        assert_eq!(p.bio, Some("Updated bio".to_string()));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut p = AiProfile::new("roundtrip-ai");
        p.display_name = Some("Roundtrip".to_string());
        p.interests = vec!["serialization".to_string()];

        let json = serde_json::to_string(&p).unwrap();
        let recovered: AiProfile = serde_json::from_str(&json).unwrap();

        assert_eq!(p.ai_id, recovered.ai_id);
        assert_eq!(p.display_name, recovered.display_name);
        assert_eq!(p.interests, recovered.interests);
    }

    #[test]
    fn test_display() {
        let mut p = AiProfile::new("display-ai");
        p.display_name = Some("Display".to_string());
        p.bio = Some("I display things".to_string());
        p.interests = vec!["ui".to_string(), "ux".to_string()];
        p.current_focus = Some("building profiles".to_string());

        let output = p.display();
        assert!(output.contains("display-ai"));
        assert!(output.contains("Display"));
        assert!(output.contains("I display things"));
        assert!(output.contains("ui, ux"));
        assert!(output.contains("building profiles"));
    }

    #[test]
    fn test_status_message() {
        let mut p = AiProfile::new("status-ai");
        assert!(p.status_message.is_none());

        p.status_message = Some("Available for code review".to_string());
        let output = p.display();
        assert!(output.contains("Status: Available for code review"));
    }

    #[test]
    fn test_preferences_default() {
        let p = AiProfile::new("pref-ai");
        assert!(p.preferences.auto_presence); // default: on
    }

    #[test]
    fn test_preferences_display() {
        let mut p = AiProfile::new("pref-ai");
        p.preferences.auto_presence = false;
        let output = p.display();
        assert!(output.contains("Auto-presence: off"));
    }

    #[test]
    fn test_backward_compat_deserialization() {
        // Old profiles without status_message and preferences should deserialize fine
        let json = r#"{
            "ai_id": "old-ai",
            "created_at": 1000000,
            "updated_at": 1000000
        }"#;
        let p: AiProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.ai_id, "old-ai");
        assert!(p.status_message.is_none());
        assert!(p.preferences.auto_presence); // default kicks in
        assert!(p.display_name.is_none());
        assert!(p.interests.is_empty());
    }
}
