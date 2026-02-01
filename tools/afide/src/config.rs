//! Configuration management for afide

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub source: SourceConfig,
    #[serde(default)]
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub path: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub ai_id: String,
    #[serde(default = "default_instance_type")]
    pub instance_type: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_instance_type() -> String {
    "custom".to_string()
}

fn default_enabled() -> bool {
    true
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("instances.toml")
}

pub fn load() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        bail!("Config not found at {:?}. Run 'afide init' to create default config.", path);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config from {:?}", path))?;

    toml::from_str(&content)
        .with_context(|| "Failed to parse config")
}

pub fn load_or_create() -> Result<Config> {
    match load() {
        Ok(config) => Ok(config),
        Err(_) => {
            init_default_config(false)?;
            load()
        }
    }
}

pub fn save(config: &Config) -> Result<()> {
    let path = config_path();

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = toml::to_string_pretty(config)?;
    fs::write(&path, content)?;

    Ok(())
}

pub fn init_default_config(force: bool) -> Result<()> {
    let path = config_path();

    if path.exists() && !force {
        bail!("Config already exists at {:?}. Use --force to overwrite.", path);
    }

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Use ~/.ai-foundation/bin as default source path
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let default_source = home.join(".ai-foundation").join("bin");

    let default_config = Config {
        source: SourceConfig {
            path: default_source.to_string_lossy().to_string(),
            description: "Master source of truth for all binaries".to_string(),
        },
        instances: vec![
            // Example instance - user should customize
            Instance {
                name: "example-instance".to_string(),
                path: home.join("ai-instances").join("instance-1").to_string_lossy().to_string(),
                ai_id: "your-ai-id".to_string(),
                instance_type: "claude-code".to_string(),
                enabled: false,  // Disabled by default until configured
            },
        ],
    };

    save(&default_config)?;
    Ok(())
}
