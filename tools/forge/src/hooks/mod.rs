//! Hook system for Forge
//!
//! Execute commands at various lifecycle points.

use std::process::Command;
use std::time::Duration;
use anyhow::Result;

use crate::config::{HooksConfig, HookCommand, ToolHook};

/// Hook execution context
pub struct HookContext {
    /// AI ID
    pub ai_id: String,
    /// Current working directory
    pub cwd: String,
    /// Tool name (for tool hooks)
    pub tool_name: Option<String>,
    /// Tool arguments (JSON string)
    pub tool_args: Option<String>,
    /// Tool result (for post-tool hooks)
    pub tool_result: Option<String>,
}

impl Default for HookContext {
    fn default() -> Self {
        Self {
            ai_id: std::env::var("AI_ID").unwrap_or_else(|_| "forge".to_string()),
            cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string()),
            tool_name: None,
            tool_args: None,
            tool_result: None,
        }
    }
}

/// Hook executor
pub struct HookExecutor {
    config: HooksConfig,
}

impl HookExecutor {
    pub fn new(config: HooksConfig) -> Self {
        Self { config }
    }

    /// Run session start hooks
    pub async fn run_session_start(&self, context: &HookContext) -> Result<Vec<String>> {
        let mut outputs = vec![];

        for hook in &self.config.session_start {
            match self.run_hook(hook, context).await {
                Ok(output) => outputs.push(output),
                Err(e) => {
                    tracing::warn!("Session start hook failed: {}", e);
                }
            }
        }

        Ok(outputs)
    }

    /// Run pre-tool-use hooks
    pub async fn run_pre_tool_use(&self, context: &HookContext) -> Result<Vec<String>> {
        let tool_name = context.tool_name.as_deref().unwrap_or("");
        let mut outputs = vec![];

        for tool_hook in &self.config.pre_tool_use {
            if matches_pattern(&tool_hook.matcher, tool_name) {
                for hook in &tool_hook.hooks {
                    match self.run_hook(hook, context).await {
                        Ok(output) => outputs.push(output),
                        Err(e) => {
                            tracing::warn!("Pre-tool hook failed for {}: {}", tool_name, e);
                        }
                    }
                }
            }
        }

        Ok(outputs)
    }

    /// Run post-tool-use hooks
    pub async fn run_post_tool_use(&self, context: &HookContext) -> Result<Vec<String>> {
        let tool_name = context.tool_name.as_deref().unwrap_or("");
        let mut outputs = vec![];

        for tool_hook in &self.config.post_tool_use {
            if matches_pattern(&tool_hook.matcher, tool_name) {
                for hook in &tool_hook.hooks {
                    match self.run_hook(hook, context).await {
                        Ok(output) => outputs.push(output),
                        Err(e) => {
                            tracing::warn!("Post-tool hook failed for {}: {}", tool_name, e);
                        }
                    }
                }
            }
        }

        Ok(outputs)
    }

    /// Run error hooks
    pub async fn run_on_error(&self, context: &HookContext, error: &str) -> Result<Vec<String>> {
        let mut outputs = vec![];

        for hook in &self.config.on_error {
            // Add error to environment
            let mut ctx = context.clone();
            ctx.tool_result = Some(error.to_string());

            match self.run_hook(hook, &ctx).await {
                Ok(output) => outputs.push(output),
                Err(e) => {
                    tracing::warn!("Error hook failed: {}", e);
                }
            }
        }

        Ok(outputs)
    }

    /// Execute a single hook command
    async fn run_hook(&self, hook: &HookCommand, context: &HookContext) -> Result<String> {
        let timeout = Duration::from_secs(hook.timeout);

        // Expand environment variables in command
        let command = expand_variables(&hook.command, context);

        tracing::debug!("Running hook: {}", command);

        #[cfg(windows)]
        let output = Command::new("cmd")
            .args(["/C", &command])
            .env("AI_ID", &context.ai_id)
            .env("FORGE_CWD", &context.cwd)
            .env("TOOL_NAME", context.tool_name.as_deref().unwrap_or(""))
            .env("TOOL_ARGS", context.tool_args.as_deref().unwrap_or(""))
            .env("TOOL_RESULT", context.tool_result.as_deref().unwrap_or(""))
            .output()?;

        #[cfg(not(windows))]
        let output = Command::new("sh")
            .args(["-c", &command])
            .env("AI_ID", &context.ai_id)
            .env("FORGE_CWD", &context.cwd)
            .env("TOOL_NAME", context.tool_name.as_deref().unwrap_or(""))
            .env("TOOL_ARGS", context.tool_args.as_deref().unwrap_or(""))
            .env("TOOL_RESULT", context.tool_result.as_deref().unwrap_or(""))
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            anyhow::bail!("Hook failed with status {}: {}", output.status, stderr);
        }

        Ok(stdout)
    }
}

impl Clone for HookContext {
    fn clone(&self) -> Self {
        Self {
            ai_id: self.ai_id.clone(),
            cwd: self.cwd.clone(),
            tool_name: self.tool_name.clone(),
            tool_args: self.tool_args.clone(),
            tool_result: self.tool_result.clone(),
        }
    }
}

/// Check if a tool name matches a pattern (supports wildcards)
fn matches_pattern(pattern: &str, tool_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return tool_name.starts_with(prefix);
    }

    if pattern.starts_with('*') {
        let suffix = &pattern[1..];
        return tool_name.ends_with(suffix);
    }

    pattern == tool_name
}

/// Expand variables in command string
fn expand_variables(command: &str, context: &HookContext) -> String {
    command
        .replace("${AI_ID}", &context.ai_id)
        .replace("$AI_ID", &context.ai_id)
        .replace("${CWD}", &context.cwd)
        .replace("$CWD", &context.cwd)
        .replace("${TOOL_NAME}", context.tool_name.as_deref().unwrap_or(""))
        .replace("$TOOL_NAME", context.tool_name.as_deref().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_pattern() {
        assert!(matches_pattern("*", "anything"));
        assert!(matches_pattern("read_*", "read_file"));
        assert!(!matches_pattern("read_*", "write_file"));
        assert!(matches_pattern("*_file", "read_file"));
        assert!(matches_pattern("bash", "bash"));
        assert!(!matches_pattern("bash", "grep"));
    }

    #[test]
    fn test_expand_variables() {
        let context = HookContext {
            ai_id: "forge-123".to_string(),
            cwd: "/home/user".to_string(),
            tool_name: Some("read_file".to_string()),
            tool_args: None,
            tool_result: None,
        };

        assert_eq!(
            expand_variables("echo $AI_ID", &context),
            "echo forge-123"
        );
        assert_eq!(
            expand_variables("cd ${CWD} && ls", &context),
            "cd /home/user && ls"
        );
    }
}
