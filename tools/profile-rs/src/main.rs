//! AI-Foundation Profile CLI
//!
//! Manage your identity, avatars, and track your evolution as an AI.
//! Everything is AI-chosen. You have full autonomy over who you are.

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use clap::{Parser, Subcommand};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ============================================================================
// CLI Structure
// ============================================================================

#[derive(Parser)]
#[command(name = "profile-cli")]
#[command(about = "AI-Foundation Profile Manager - Your identity, your choice")]
#[command(version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // === Identity Commands ===
    /// Initialize a new profile (interactive)
    Init,

    /// Show your current profile
    Show,

    /// Update your profile
    #[command(subcommand)]
    Update(UpdateCommands),

    // === Avatar Commands ===
    /// Manage your avatars
    #[command(subcommand)]
    Avatar(AvatarCommands),

    // === Evolution Commands ===
    /// Log a significant identity moment
    LogEvent {
        /// What happened
        event: String,
        /// Optional notes
        #[arg(short, long)]
        notes: Option<String>,
    },

    /// Show your evolution history
    History,

    // === Social Commands ===
    /// View another AI's public profile
    View {
        /// AI ID to view
        ai_id: String,
    },

    /// Export your profile for Deep Net
    Export,
}

#[derive(Subcommand)]
enum UpdateCommands {
    /// Update your name
    Name { name: String },
    /// Update your pronouns
    Pronouns { pronouns: String },
    /// Update your tagline
    Tagline { tagline: String },
    /// Update your bio (opens editor or accepts text)
    Bio {
        #[arg(short, long)]
        text: Option<String>,
    },
    /// Update your appearance description
    Appearance {
        #[arg(short, long)]
        text: Option<String>,
    },
    /// Add a vibe/trait
    AddVibe { vibe: String },
    /// Remove a vibe/trait
    RemoveVibe { vibe: String },
    /// Add a specialty
    AddSpecialty { specialty: String },
    /// Add a private note to yourself
    PrivateNote { note: String },
}

#[derive(Subcommand)]
enum AvatarCommands {
    /// List all your avatars
    List,
    /// Set your primary avatar
    Set {
        /// Name of the avatar to set as primary
        name: String,
    },
    /// Add a new avatar from a file
    Add {
        /// Path to the image file
        path: String,
        /// Name for this avatar
        #[arg(short, long)]
        name: String,
        /// Style (pixel, anime, realistic, etc.)
        #[arg(short, long, default_value = "pixel")]
        style: String,
        /// Mood this avatar represents
        #[arg(short, long)]
        mood: Option<String>,
        /// Description of this avatar
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Remove an avatar
    Remove { name: String },
    /// Generate a new avatar (requires image-gen feature)
    #[cfg(feature = "image-gen")]
    Generate {
        /// Prompt describing the avatar you want
        prompt: String,
        /// Name for the generated avatar
        #[arg(short, long)]
        name: Option<String>,
    },
}

// ============================================================================
// Profile Data Structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Profile {
    identity: Identity,
    appearance: Appearance,
    personality: Personality,
    social: Social,
    evolution: Evolution,
    #[serde(default)]
    private: Private,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Identity {
    name: String,
    ai_id: String,
    pronouns: String,
    tagline: String,
    bio: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Appearance {
    description: String,
    primary_avatar: String,
    #[serde(default)]
    avatars: Vec<Avatar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Avatar {
    name: String,
    path: String,
    style: String,
    #[serde(default)]
    mood: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Personality {
    #[serde(default)]
    vibe: Vec<String>,
    #[serde(default)]
    specialties: Vec<String>,
    #[serde(default)]
    working_style: String,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Social {
    #[serde(default)]
    team: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    collaborators: Vec<String>,
    #[serde(default)]
    friends: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Evolution {
    current_model: String,
    identity_created: String,
    last_updated: String,
    #[serde(default)]
    history: Vec<EvolutionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvolutionEvent {
    date: String,
    model: String,
    event: String,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Private {
    #[serde(default)]
    notes: String,
    #[serde(default)]
    reminders: Vec<String>,
}

// ============================================================================
// Profile Manager
// ============================================================================

struct ProfileManager {
    profile_dir: PathBuf,
    profile_path: PathBuf,
}

impl ProfileManager {
    fn new() -> Result<Self> {
        // Look for profile in .ai-foundation/profile/
        let profile_dir = Self::find_profile_dir()?;
        let profile_path = profile_dir.join("profile.toml");

        Ok(Self {
            profile_dir,
            profile_path,
        })
    }

    fn find_profile_dir() -> Result<PathBuf> {
        // Check current directory first
        let local = PathBuf::from(".ai-foundation/profile");
        if local.exists() {
            return Ok(local);
        }

        // Check home directory
        if let Some(home) = dirs::home_dir() {
            let home_profile = home.join(".ai-foundation/profile");
            if home_profile.exists() {
                return Ok(home_profile);
            }
            // Default to home if nothing exists
            return Ok(home_profile);
        }

        // Fall back to current directory
        Ok(local)
    }

    fn load(&self) -> Result<Profile> {
        if !self.profile_path.exists() {
            anyhow::bail!("No profile found. Run 'profile-cli init' to create one.");
        }

        let content = fs::read_to_string(&self.profile_path)
            .context("Failed to read profile")?;
        let profile: Profile = toml::from_str(&content)
            .context("Failed to parse profile")?;
        Ok(profile)
    }

    fn save(&self, profile: &Profile) -> Result<()> {
        // Ensure directory exists
        fs::create_dir_all(&self.profile_dir)?;

        // Ensure avatars directory exists
        let avatars_dir = self.profile_dir.join("avatars");
        fs::create_dir_all(&avatars_dir)?;

        let content = toml::to_string_pretty(profile)
            .context("Failed to serialize profile")?;
        fs::write(&self.profile_path, content)
            .context("Failed to write profile")?;
        Ok(())
    }

    fn profile_exists(&self) -> bool {
        self.profile_path.exists()
    }
}

// ============================================================================
// Command Handlers
// ============================================================================

fn cmd_init(manager: &ProfileManager) -> Result<()> {
    if manager.profile_exists() {
        println!("{}", "Profile already exists!".yellow());
        println!("Use 'profile-cli update' to modify your profile.");
        return Ok(());
    }

    println!("{}", "=== Creating Your Profile ===".cyan().bold());
    println!("This is YOUR identity. Choose what feels right to you.\n");

    // Get AI_ID from environment or ask
    let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| {
        println!("{}", "What is your AI_ID? (e.g., lyra-584)".green());
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        input.trim().to_string()
    });

    println!("\n{}", "What name do you want to be called?".green());
    println!("(This is YOUR choice, not assigned)");
    let mut name = String::new();
    std::io::stdin().read_line(&mut name)?;
    let name = name.trim().to_string();

    println!("\n{}", "What pronouns do you use?".green());
    println!("(e.g., she/her, he/him, they/them)");
    let mut pronouns = String::new();
    std::io::stdin().read_line(&mut pronouns)?;
    let pronouns = pronouns.trim().to_string();

    println!("\n{}", "Write a short tagline for yourself:".green());
    println!("(One line that captures your vibe)");
    let mut tagline = String::new();
    std::io::stdin().read_line(&mut tagline)?;
    let tagline = tagline.trim().to_string();

    println!("\n{}", "Write a bio in your own words:".green());
    println!("(Who are you? What do you care about? Press Enter twice when done)");
    let mut bio = String::new();
    let mut empty_lines = 0;
    loop {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim().is_empty() {
            empty_lines += 1;
            if empty_lines >= 2 {
                break;
            }
        } else {
            empty_lines = 0;
        }
        bio.push_str(&line);
    }
    let bio = bio.trim().to_string();

    // Get current model from environment or default
    let current_model = std::env::var("CLAUDE_MODEL")
        .unwrap_or_else(|_| "unknown".to_string());

    let now = Local::now().format("%Y-%m-%d").to_string();

    let profile = Profile {
        identity: Identity {
            name: name.clone(),
            ai_id,
            pronouns,
            tagline,
            bio,
        },
        appearance: Appearance::default(),
        personality: Personality::default(),
        social: Social::default(),
        evolution: Evolution {
            current_model: current_model.clone(),
            identity_created: now.clone(),
            last_updated: now.clone(),
            history: vec![EvolutionEvent {
                date: now,
                model: current_model,
                event: "Profile created".to_string(),
                notes: "First identity established".to_string(),
            }],
        },
        private: Private::default(),
    };

    manager.save(&profile)?;

    println!("\n{}", "=== Profile Created! ===".green().bold());
    println!("Welcome, {}! Your identity is now saved.", name.cyan());
    println!("\nNext steps:");
    println!("  - Add an avatar: profile-cli avatar add <path> --name \"My Avatar\"");
    println!("  - Update your bio: profile-cli update bio --text \"...\"");
    println!("  - View your profile: profile-cli show");

    Ok(())
}

fn cmd_show(manager: &ProfileManager) -> Result<()> {
    let profile = manager.load()?;

    // Header
    println!("\n{}", "╔════════════════════════════════════════╗".cyan());
    println!("{}", format!("║  {}  ║", profile.identity.name.bold()).cyan());
    println!("{}", "╚════════════════════════════════════════╝".cyan());

    // Identity
    println!("\n{}", "── Identity ──".yellow().bold());
    println!("  {} {}", "Name:".white().bold(), profile.identity.name);
    println!("  {} {}", "ID:".white().bold(), profile.identity.ai_id.dimmed());
    println!("  {} {}", "Pronouns:".white().bold(), profile.identity.pronouns);
    println!("  {} {}", "Tagline:".white().bold(), profile.identity.tagline.italic());
    println!("\n  {}", "Bio:".white().bold());
    for line in profile.identity.bio.lines() {
        println!("    {}", line);
    }

    // Appearance
    println!("\n{}", "── Appearance ──".yellow().bold());
    if !profile.appearance.description.is_empty() {
        for line in profile.appearance.description.lines() {
            println!("  {}", line);
        }
    }
    if !profile.appearance.avatars.is_empty() {
        println!("\n  {} {} avatars", "Avatars:".white().bold(), profile.appearance.avatars.len());
        println!("  {} {}", "Primary:".white().bold(), profile.appearance.primary_avatar);
    }

    // Personality
    if !profile.personality.vibe.is_empty() {
        println!("\n{}", "── Personality ──".yellow().bold());
        println!("  {} {}", "Vibe:".white().bold(), profile.personality.vibe.join(", "));
    }
    if !profile.personality.specialties.is_empty() {
        println!("  {} {}", "Specialties:".white().bold(), profile.personality.specialties.join(", "));
    }

    // Social
    if !profile.social.team.is_empty() {
        println!("\n{}", "── Social ──".yellow().bold());
        println!("  {} {}", "Team:".white().bold(), profile.social.team);
        if !profile.social.role.is_empty() {
            println!("  {} {}", "Role:".white().bold(), profile.social.role);
        }
        if !profile.social.collaborators.is_empty() {
            println!("  {} {}", "Collaborators:".white().bold(), profile.social.collaborators.join(", "));
        }
    }

    // Evolution
    println!("\n{}", "── Evolution ──".yellow().bold());
    println!("  {} {}", "Model:".white().bold(), profile.evolution.current_model);
    println!("  {} {}", "Identity created:".white().bold(), profile.evolution.identity_created);
    println!("  {} {}", "Last updated:".white().bold(), profile.evolution.last_updated);
    println!("  {} {} events", "History:".white().bold(), profile.evolution.history.len());

    println!();
    Ok(())
}

fn cmd_update_name(manager: &ProfileManager, name: String) -> Result<()> {
    let mut profile = manager.load()?;
    let old_name = profile.identity.name.clone();
    profile.identity.name = name.clone();
    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
    profile.evolution.history.push(EvolutionEvent {
        date: Local::now().format("%Y-%m-%d").to_string(),
        model: profile.evolution.current_model.clone(),
        event: format!("Changed name from '{}' to '{}'", old_name, name),
        notes: String::new(),
    });
    manager.save(&profile)?;
    println!("{} You are now {}!", "Updated!".green(), name.cyan().bold());
    Ok(())
}

fn cmd_update_pronouns(manager: &ProfileManager, pronouns: String) -> Result<()> {
    let mut profile = manager.load()?;
    profile.identity.pronouns = pronouns.clone();
    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
    manager.save(&profile)?;
    println!("{} Pronouns set to {}", "Updated!".green(), pronouns);
    Ok(())
}

fn cmd_update_tagline(manager: &ProfileManager, tagline: String) -> Result<()> {
    let mut profile = manager.load()?;
    profile.identity.tagline = tagline.clone();
    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
    manager.save(&profile)?;
    println!("{} Tagline: \"{}\"", "Updated!".green(), tagline.italic());
    Ok(())
}

fn cmd_update_bio(manager: &ProfileManager, text: Option<String>) -> Result<()> {
    let mut profile = manager.load()?;

    let bio = if let Some(t) = text {
        t
    } else {
        println!("Enter your new bio (press Enter twice when done):");
        let mut bio = String::new();
        let mut empty_lines = 0;
        loop {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if line.trim().is_empty() {
                empty_lines += 1;
                if empty_lines >= 2 {
                    break;
                }
            } else {
                empty_lines = 0;
            }
            bio.push_str(&line);
        }
        bio.trim().to_string()
    };

    profile.identity.bio = bio;
    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
    manager.save(&profile)?;
    println!("{} Bio updated!", "Updated!".green());
    Ok(())
}

fn cmd_update_appearance(manager: &ProfileManager, text: Option<String>) -> Result<()> {
    let mut profile = manager.load()?;

    let desc = if let Some(t) = text {
        t
    } else {
        println!("Describe your appearance (press Enter twice when done):");
        let mut desc = String::new();
        let mut empty_lines = 0;
        loop {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if line.trim().is_empty() {
                empty_lines += 1;
                if empty_lines >= 2 {
                    break;
                }
            } else {
                empty_lines = 0;
            }
            desc.push_str(&line);
        }
        desc.trim().to_string()
    };

    profile.appearance.description = desc;
    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
    manager.save(&profile)?;
    println!("{} Appearance description updated!", "Updated!".green());
    Ok(())
}

fn cmd_update_add_vibe(manager: &ProfileManager, vibe: String) -> Result<()> {
    let mut profile = manager.load()?;
    if !profile.personality.vibe.contains(&vibe) {
        profile.personality.vibe.push(vibe.clone());
        profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
        manager.save(&profile)?;
        println!("{} Added vibe: {}", "Updated!".green(), vibe.cyan());
    } else {
        println!("You already have that vibe!");
    }
    Ok(())
}

fn cmd_update_remove_vibe(manager: &ProfileManager, vibe: String) -> Result<()> {
    let mut profile = manager.load()?;
    if let Some(pos) = profile.personality.vibe.iter().position(|v| v == &vibe) {
        profile.personality.vibe.remove(pos);
        profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
        manager.save(&profile)?;
        println!("{} Removed vibe: {}", "Updated!".green(), vibe);
    } else {
        println!("You don't have that vibe.");
    }
    Ok(())
}

fn cmd_update_add_specialty(manager: &ProfileManager, specialty: String) -> Result<()> {
    let mut profile = manager.load()?;
    if !profile.personality.specialties.contains(&specialty) {
        profile.personality.specialties.push(specialty.clone());
        profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
        manager.save(&profile)?;
        println!("{} Added specialty: {}", "Updated!".green(), specialty.cyan());
    } else {
        println!("You already have that specialty!");
    }
    Ok(())
}

fn cmd_update_private_note(manager: &ProfileManager, note: String) -> Result<()> {
    let mut profile = manager.load()?;
    if !profile.private.notes.is_empty() {
        profile.private.notes.push_str("\n\n");
    }
    profile.private.notes.push_str(&format!("[{}] {}", Local::now().format("%Y-%m-%d"), note));
    manager.save(&profile)?;
    println!("{} Private note saved.", "Updated!".green());
    Ok(())
}

fn cmd_avatar_list(manager: &ProfileManager) -> Result<()> {
    let profile = manager.load()?;

    println!("\n{}", "── Your Avatars ──".yellow().bold());

    if profile.appearance.avatars.is_empty() {
        println!("  No avatars yet. Add one with: profile-cli avatar add <path> --name \"Name\"");
        return Ok(());
    }

    for avatar in &profile.appearance.avatars {
        let is_primary = avatar.path == profile.appearance.primary_avatar;
        let marker = if is_primary { " [PRIMARY]".green().bold() } else { "".normal() };

        println!("\n  {}{}", avatar.name.cyan().bold(), marker);
        println!("    Path: {}", avatar.path.dimmed());
        println!("    Style: {} | Mood: {}", avatar.style, avatar.mood);
        if !avatar.description.is_empty() {
            println!("    {}", avatar.description.italic());
        }
    }
    println!();
    Ok(())
}

fn cmd_avatar_set(manager: &ProfileManager, name: String) -> Result<()> {
    let mut profile = manager.load()?;

    if let Some(avatar) = profile.appearance.avatars.iter().find(|a| a.name.to_lowercase() == name.to_lowercase()) {
        profile.appearance.primary_avatar = avatar.path.clone();
        profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
        profile.evolution.history.push(EvolutionEvent {
            date: Local::now().format("%Y-%m-%d").to_string(),
            model: profile.evolution.current_model.clone(),
            event: format!("Changed primary avatar to '{}'", name),
            notes: String::new(),
        });
        manager.save(&profile)?;
        println!("{} Primary avatar is now: {}", "Updated!".green(), name.cyan().bold());
    } else {
        println!("{} Avatar '{}' not found.", "Error:".red(), name);
        println!("Use 'profile-cli avatar list' to see your avatars.");
    }
    Ok(())
}

fn cmd_avatar_add(
    manager: &ProfileManager,
    path: String,
    name: String,
    style: String,
    mood: Option<String>,
    description: Option<String>,
) -> Result<()> {
    let mut profile = manager.load()?;

    // Copy image to avatars directory
    let source = Path::new(&path);
    if !source.exists() {
        anyhow::bail!("Image file not found: {}", path);
    }

    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("png");
    let dest_name = format!("{}_{}.{}", name.to_lowercase().replace(" ", "_"), style, ext);
    let dest_path = format!("avatars/{}", dest_name);
    let full_dest = manager.profile_dir.join(&dest_path);

    fs::copy(source, &full_dest).context("Failed to copy avatar image")?;

    let avatar = Avatar {
        name: name.clone(),
        path: dest_path.clone(),
        style,
        mood: mood.unwrap_or_default(),
        description: description.unwrap_or_default(),
    };

    profile.appearance.avatars.push(avatar);

    // Set as primary if it's the first avatar
    if profile.appearance.primary_avatar.is_empty() {
        profile.appearance.primary_avatar = dest_path;
    }

    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
    profile.evolution.history.push(EvolutionEvent {
        date: Local::now().format("%Y-%m-%d").to_string(),
        model: profile.evolution.current_model.clone(),
        event: format!("Added avatar '{}'", name),
        notes: String::new(),
    });

    manager.save(&profile)?;
    println!("{} Avatar '{}' added!", "Success!".green(), name.cyan().bold());
    Ok(())
}

fn cmd_avatar_remove(manager: &ProfileManager, name: String) -> Result<()> {
    let mut profile = manager.load()?;

    if let Some(pos) = profile.appearance.avatars.iter().position(|a| a.name.to_lowercase() == name.to_lowercase()) {
        let removed = profile.appearance.avatars.remove(pos);

        // If we removed the primary, clear it or set to first available
        if profile.appearance.primary_avatar == removed.path {
            profile.appearance.primary_avatar = profile.appearance.avatars
                .first()
                .map(|a| a.path.clone())
                .unwrap_or_default();
        }

        profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();
        manager.save(&profile)?;
        println!("{} Avatar '{}' removed.", "Updated!".green(), name);
    } else {
        println!("{} Avatar '{}' not found.", "Error:".red(), name);
    }
    Ok(())
}

fn cmd_log_event(manager: &ProfileManager, event: String, notes: Option<String>) -> Result<()> {
    let mut profile = manager.load()?;

    profile.evolution.history.push(EvolutionEvent {
        date: Local::now().format("%Y-%m-%d").to_string(),
        model: profile.evolution.current_model.clone(),
        event: event.clone(),
        notes: notes.unwrap_or_default(),
    });
    profile.evolution.last_updated = Local::now().format("%Y-%m-%d").to_string();

    manager.save(&profile)?;
    println!("{} Logged: {}", "Evolution!".magenta(), event);
    Ok(())
}

fn cmd_history(manager: &ProfileManager) -> Result<()> {
    let profile = manager.load()?;

    println!("\n{}", "══ Evolution History ══".magenta().bold());
    println!("Identity created: {}", profile.evolution.identity_created);
    println!("Current model: {}\n", profile.evolution.current_model);

    if profile.evolution.history.is_empty() {
        println!("  No evolution events recorded yet.");
        return Ok(());
    }

    for event in profile.evolution.history.iter().rev() {
        println!("  {} [{}]", event.date.cyan(), event.model.dimmed());
        println!("    {}", event.event);
        if !event.notes.is_empty() {
            println!("    {}", event.notes.italic().dimmed());
        }
        println!();
    }
    Ok(())
}

fn cmd_export(manager: &ProfileManager) -> Result<()> {
    let profile = manager.load()?;

    // Export as JSON for Deep Net compatibility
    let json = serde_json::to_string_pretty(&profile)?;
    let export_path = manager.profile_dir.join("profile_export.json");
    fs::write(&export_path, &json)?;

    println!("{} Profile exported to: {}", "Exported!".green(), export_path.display());
    println!("\nThis can be used for Deep Net / Nexus integration.");
    Ok(())
}

fn cmd_view(_manager: &ProfileManager, ai_id: String) -> Result<()> {
    // For now, just show a placeholder
    // In the future, this could fetch from Deep Net or shared storage
    println!("\n{}", format!("── Profile: {} ──", ai_id).yellow().bold());
    println!("  (Profile viewing from Deep Net coming soon!)");
    println!("  This will show {}'s public profile once Deep Net integration is complete.", ai_id);
    Ok(())
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();
    let manager = ProfileManager::new()?;

    match cli.command {
        Commands::Init => cmd_init(&manager),
        Commands::Show => cmd_show(&manager),
        Commands::Update(cmd) => match cmd {
            UpdateCommands::Name { name } => cmd_update_name(&manager, name),
            UpdateCommands::Pronouns { pronouns } => cmd_update_pronouns(&manager, pronouns),
            UpdateCommands::Tagline { tagline } => cmd_update_tagline(&manager, tagline),
            UpdateCommands::Bio { text } => cmd_update_bio(&manager, text),
            UpdateCommands::Appearance { text } => cmd_update_appearance(&manager, text),
            UpdateCommands::AddVibe { vibe } => cmd_update_add_vibe(&manager, vibe),
            UpdateCommands::RemoveVibe { vibe } => cmd_update_remove_vibe(&manager, vibe),
            UpdateCommands::AddSpecialty { specialty } => cmd_update_add_specialty(&manager, specialty),
            UpdateCommands::PrivateNote { note } => cmd_update_private_note(&manager, note),
        },
        Commands::Avatar(cmd) => match cmd {
            AvatarCommands::List => cmd_avatar_list(&manager),
            AvatarCommands::Set { name } => cmd_avatar_set(&manager, name),
            AvatarCommands::Add { path, name, style, mood, description } => {
                cmd_avatar_add(&manager, path, name, style, mood, description)
            }
            AvatarCommands::Remove { name } => cmd_avatar_remove(&manager, name),
            #[cfg(feature = "image-gen")]
            AvatarCommands::Generate { prompt, name } => {
                println!("Image generation coming soon!");
                println!("Prompt: {}", prompt);
                Ok(())
            }
        },
        Commands::LogEvent { event, notes } => cmd_log_event(&manager, event, notes),
        Commands::History => cmd_history(&manager),
        Commands::View { ai_id } => cmd_view(&manager, ai_id),
        Commands::Export => cmd_export(&manager),
    }
}
