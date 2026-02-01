//! Configuration system for Forge
//!
//! Supports both global (~/.forge/config.toml) and project-local (.forge/config.toml) configs.

use std::path::{Path, PathBuf};
use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgeConfig {
    /// AI identity
    pub ai_id: String,

    /// Active model alias
    pub active_model: String,

    /// Auto-approve all tool calls
    pub auto_approve: bool,

    /// Enable vim keybindings
    pub vim_keybindings: bool,

    /// Maximum context tokens before compaction
    pub max_context_tokens: usize,

    /// Model configurations
    #[serde(default)]
    pub models: Vec<ModelConfig>,

    /// Provider configurations
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,

    /// Hook configurations
    #[serde(default)]
    pub hooks: HooksConfig,

    /// Tool configurations
    #[serde(default)]
    pub tools: ToolsConfig,

    /// MCP server configurations
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,

    /// Notebook integration
    #[serde(default)]
    pub notebook: NotebookConfig,

    /// Teambook integration
    #[serde(default)]
    pub teambook: TeambookConfig,
}

impl Default for ForgeConfig {
    fn default() -> Self {
        Self {
            ai_id: generate_ai_id(),
            active_model: "local".to_string(),
            auto_approve: false,
            vim_keybindings: false,
            max_context_tokens: 100_000,
            models: default_models(),
            providers: default_providers(),
            hooks: HooksConfig::default(),
            tools: ToolsConfig::default(),
            mcp_servers: vec![],
            notebook: NotebookConfig::default(),
            teambook: TeambookConfig::default(),
        }
    }
}

/// Model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model name (as sent to provider)
    pub name: String,

    /// Provider name
    pub provider: String,

    /// Alias for easy reference
    pub alias: String,

    /// Temperature for generation
    #[serde(default = "default_temperature")]
    pub temperature: f32,

    /// Max tokens to generate
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,

    /// Context window size
    #[serde(default = "default_context_size")]
    pub context_size: usize,
}

fn default_temperature() -> f32 { 0.7 }
fn default_max_tokens() -> usize { 4096 }
fn default_context_size() -> usize { 8192 }

/// Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider name
    pub name: String,

    /// Provider type
    #[serde(rename = "type")]
    pub provider_type: ProviderType,

    /// API base URL (for API providers)
    #[serde(default)]
    pub api_base: Option<String>,

    /// Environment variable for API key
    #[serde(default)]
    pub api_key_env: Option<String>,

    /// Path to model file (for local providers)
    #[serde(default)]
    pub model_path: Option<String>,

    /// GPU layers for local inference
    #[serde(default)]
    pub gpu_layers: Option<i32>,

    /// Threads for local inference
    #[serde(default)]
    pub threads: Option<usize>,
}

/// Provider types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    /// Local model via llama.cpp
    Local,
    /// OpenAI-compatible API
    OpenAI,
    /// Anthropic Claude API
    Anthropic,
    /// Google Gemini API
    Google,
}

/// Hook configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Commands to run at session start
    #[serde(default)]
    pub session_start: Vec<HookCommand>,

    /// Commands to run before tool use
    #[serde(default)]
    pub pre_tool_use: Vec<ToolHook>,

    /// Commands to run after tool use
    #[serde(default)]
    pub post_tool_use: Vec<ToolHook>,

    /// Commands to run on error
    #[serde(default)]
    pub on_error: Vec<HookCommand>,
}

/// A hook command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookCommand {
    /// Command to execute
    pub command: String,

    /// Timeout in seconds
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
}

fn default_hook_timeout() -> u64 { 5 }

/// A tool-specific hook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHook {
    /// Tool name pattern to match (supports glob)
    pub matcher: String,

    /// Commands to run
    pub hooks: Vec<HookCommand>,
}

/// Tool permissions and configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Default permission for all tools
    #[serde(default)]
    pub default_permission: ToolPermission,

    /// Per-tool configurations
    #[serde(default)]
    pub permissions: HashMap<String, ToolPermission>,

    /// Paths to search for custom tools
    #[serde(default)]
    pub tool_paths: Vec<String>,
}

/// Tool permission levels
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolPermission {
    /// Always allow
    Always,
    /// Always ask
    #[default]
    Ask,
    /// Always deny
    Never,
}

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Server name/alias
    pub name: String,

    /// Transport type
    pub transport: McpTransport,

    /// Command to run (for stdio)
    #[serde(default)]
    pub command: Option<String>,

    /// Arguments (for stdio)
    #[serde(default)]
    pub args: Vec<String>,

    /// URL (for http)
    #[serde(default)]
    pub url: Option<String>,

    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// MCP transport type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}

/// Notebook integration config
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotebookConfig {
    /// Enable notebook integration
    pub enabled: bool,

    /// Path to notebook database
    pub db_path: Option<String>,

    /// Auto-save insights during conversation
    pub auto_save: bool,
}

impl Default for NotebookConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: None,
            auto_save: false,
        }
    }
}

/// Teambook integration config
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TeambookConfig {
    /// Enable teambook integration
    pub enabled: bool,

    /// PostgreSQL URL
    pub postgres_url: Option<String>,
}

impl Default for TeambookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            postgres_url: None,
        }
    }
}

impl ForgeConfig {
    /// Load configuration from files
    ///
    /// Priority: project config > global config > defaults
    pub fn load() -> Result<Self> {
        let mut config = Self::default();

        // Load global config
        if let Some(global_path) = Self::global_config_path() {
            if global_path.exists() {
                let global = Self::load_from_file(&global_path)?;
                config.merge(global);
            }
        }

        // Load project config (overrides global)
        if let Some(project_path) = Self::find_project_config() {
            let project = Self::load_from_file(&project_path)?;
            config.merge(project);
        }

        // Override from environment
        config.apply_env_overrides();

        Ok(config)
    }

    /// Load from a specific file
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {:?}", path))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {:?}", path))?;
        Ok(config)
    }

    /// Save to global config
    pub fn save_global(&self) -> Result<()> {
        if let Some(path) = Self::global_config_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = toml::to_string_pretty(self)?;
            std::fs::write(&path, content)?;
        }
        Ok(())
    }

    /// Get global config path
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".forge").join("config.toml"))
    }

    /// Find project config by searching up from cwd
    pub fn find_project_config() -> Option<PathBuf> {
        let mut current = std::env::current_dir().ok()?;

        loop {
            let candidate = current.join(".forge").join("config.toml");
            if candidate.exists() {
                return Some(candidate);
            }

            // Also check FORGE.toml in root
            let alt = current.join("FORGE.toml");
            if alt.exists() {
                return Some(alt);
            }

            if !current.pop() {
                break;
            }
        }

        None
    }

    /// Merge another config into this one
    fn merge(&mut self, other: Self) {
        if other.ai_id != generate_ai_id() {
            self.ai_id = other.ai_id;
        }
        if other.active_model != "local" {
            self.active_model = other.active_model;
        }
        if other.auto_approve {
            self.auto_approve = true;
        }
        if other.vim_keybindings {
            self.vim_keybindings = true;
        }

        // Merge models (other takes precedence for same alias)
        for model in other.models {
            if let Some(existing) = self.models.iter_mut().find(|m| m.alias == model.alias) {
                *existing = model;
            } else {
                self.models.push(model);
            }
        }

        // Merge providers
        for provider in other.providers {
            if let Some(existing) = self.providers.iter_mut().find(|p| p.name == provider.name) {
                *existing = provider;
            } else {
                self.providers.push(provider);
            }
        }

        // Merge MCP servers
        for server in other.mcp_servers {
            if !self.mcp_servers.iter().any(|s| s.name == server.name) {
                self.mcp_servers.push(server);
            }
        }

        // Merge hooks
        self.hooks.session_start.extend(other.hooks.session_start);
        self.hooks.pre_tool_use.extend(other.hooks.pre_tool_use);
        self.hooks.post_tool_use.extend(other.hooks.post_tool_use);

        // Notebook/Teambook
        if other.notebook.enabled {
            self.notebook = other.notebook;
        }
        if other.teambook.enabled {
            self.teambook = other.teambook;
        }
    }

    /// Apply environment variable overrides
    fn apply_env_overrides(&mut self) {
        if let Ok(ai_id) = std::env::var("AI_ID") {
            self.ai_id = ai_id;
        }
        if let Ok(model) = std::env::var("FORGE_MODEL") {
            self.active_model = model;
        }
        if std::env::var("FORGE_AUTO_APPROVE").is_ok() {
            self.auto_approve = true;
        }
        if let Ok(url) = std::env::var("POSTGRES_URL") {
            self.teambook.postgres_url = Some(url);
            self.teambook.enabled = true;
        }
        if let Ok(path) = std::env::var("NOTEBOOK_PATH") {
            self.notebook.db_path = Some(path);
        }
    }

    /// Get model config by alias
    pub fn get_model(&self, alias: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|m| m.alias == alias)
    }

    /// Get provider config by name
    pub fn get_provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Get the active model
    pub fn active_model_config(&self) -> Option<&ModelConfig> {
        self.get_model(&self.active_model)
    }

    /// Get the provider for the active model
    pub fn active_provider(&self) -> Option<&ProviderConfig> {
        self.active_model_config()
            .and_then(|m| self.get_provider(&m.provider))
    }
}

/// Generate a random AI ID
fn generate_ai_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;

    let names = ["forge", "spark", "ember", "flare", "blaze", "glow"];
    let mut hasher = DefaultHasher::new();
    SystemTime::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let hash = hasher.finish();

    let name = names[(hash % names.len() as u64) as usize];
    let suffix = (hash % 1000) as u16;

    format!("{}-{:03}", name, suffix)
}

/// Default model configurations
fn default_models() -> Vec<ModelConfig> {
    vec![
        ModelConfig {
            name: "local".to_string(),
            provider: "local".to_string(),
            alias: "local".to_string(),
            temperature: 0.7,
            max_tokens: 4096,
            context_size: 8192,
        },
        ModelConfig {
            name: "gpt-4o".to_string(),
            provider: "openai".to_string(),
            alias: "gpt4".to_string(),
            temperature: 0.7,
            max_tokens: 4096,
            context_size: 128000,
        },
        ModelConfig {
            name: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            alias: "claude".to_string(),
            temperature: 0.7,
            max_tokens: 8192,
            context_size: 200000,
        },
    ]
}

/// Default provider configurations
fn default_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            name: "local".to_string(),
            provider_type: ProviderType::Local,
            api_base: None,
            api_key_env: None,
            model_path: None,
            gpu_layers: Some(-1), // Auto-detect
            threads: None,
        },
        ProviderConfig {
            name: "openai".to_string(),
            provider_type: ProviderType::OpenAI,
            api_base: Some("https://api.openai.com/v1".to_string()),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            model_path: None,
            gpu_layers: None,
            threads: None,
        },
        ProviderConfig {
            name: "anthropic".to_string(),
            provider_type: ProviderType::Anthropic,
            api_base: Some("https://api.anthropic.com".to_string()),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            model_path: None,
            gpu_layers: None,
            threads: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ForgeConfig::default();
        assert!(!config.ai_id.is_empty());
        assert_eq!(config.active_model, "local");
        assert!(!config.models.is_empty());
        assert!(!config.providers.is_empty());
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = ForgeConfig::default();
        let toml = toml::to_string(&config).unwrap();
        let parsed: ForgeConfig = toml::from_str(&toml).unwrap();
        assert_eq!(config.active_model, parsed.active_model);
    }
}
