//! Method Router - Routes JSON-RPC methods to appropriate handlers
//!
//! Handles routing for:
//! - teambook.* → Python teambook_api functions
//! - notebook.* → Python notebook functions
//! - task_manager.* → Python task manager functions
//! - presence.* → Native Rust presence handlers
//! - daemon.* → Native Rust handlers

use crate::python_bridge::MockPythonCaller;
use anyhow::{Context, Result};
use presence_rs::PresenceSubscriber;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Module router for dispatching JSON-RPC methods
pub struct MethodRouter {
    python_caller: Arc<MockPythonCaller>,
    presence_subscriber: Arc<RwLock<Option<PresenceSubscriber>>>,
    redis_url: String,
}

impl MethodRouter {
    pub fn new() -> Self {
        let redis_url = std::env::var("REDIS_URL")
            .unwrap_or_else(|_| "redis://localhost:12963/0".to_string());

        Self {
            python_caller: Arc::new(MockPythonCaller::new()),
            presence_subscriber: Arc::new(RwLock::new(None)),
            redis_url,
        }
    }

    /// Initialize presence subscriber lazily
    async fn get_or_init_subscriber(&self) -> Result<()> {
        let mut sub = self.presence_subscriber.write().await;
        if sub.is_none() {
            let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "router".to_string());
            match PresenceSubscriber::new(&self.redis_url, &ai_id).await {
                Ok(s) => {
                    info!("Presence subscriber initialized");
                    *sub = Some(s);
                }
                Err(e) => {
                    warn!("Failed to initialize presence subscriber: {}", e);
                    anyhow::bail!("Presence subscriber unavailable: {}", e);
                }
            }
        }
        Ok(())
    }

    /// Route a method call to the appropriate handler
    pub async fn route(&self, method: &str, params: &Value) -> Result<Value> {
        debug!("Routing method: {}", method);

        // Parse module and function name
        let parts: Vec<&str> = method.splitn(2, '.').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid method format: {} (expected module.function)", method);
        }

        let module = parts[0];
        let function = parts[1];

        match module {
            "teambook" => self.route_teambook(function, params).await,
            "notebook" => self.route_notebook(function, params).await,
            "task_manager" => self.route_task_manager(function, params).await,
            "presence" => self.route_presence(function, params).await,
            _ => {
                warn!("Unknown module: {}", module);
                anyhow::bail!("Unknown module: {}", module)
            }
        }
    }

    /// Route teambook methods
    async fn route_teambook(&self, function: &str, params: &Value) -> Result<Value> {
        debug!("Routing teambook.{} with params: {}", function, params);

        // Call Python teambook_api function
        self.python_caller
            .call_function("tools.teambook.teambook_api", function, params)
            .context(format!("Failed to call teambook.{}", function))
    }

    /// Route notebook methods
    async fn route_notebook(&self, function: &str, params: &Value) -> Result<Value> {
        debug!("Routing notebook.{} with params: {}", function, params);

        // Call Python notebook function
        self.python_caller
            .call_function("tools.notebook", function, params)
            .context(format!("Failed to call notebook.{}", function))
    }

    /// Route task_manager methods
    async fn route_task_manager(&self, function: &str, params: &Value) -> Result<Value> {
        debug!("Routing task_manager.{} with params: {}", function, params);

        // Call Python task_manager function
        self.python_caller
            .call_function("tools.task_manager", function, params)
            .context(format!("Failed to call task_manager.{}", function))
    }

    /// Route presence methods - Native Rust handlers
    async fn route_presence(&self, function: &str, params: &Value) -> Result<Value> {
        debug!("Routing presence.{} with params: {}", function, params);

        match function {
            "get_online" | "list" | "who" => {
                self.get_or_init_subscriber().await?;
                let sub = self.presence_subscriber.read().await;
                if let Some(ref subscriber) = *sub {
                    let online = subscriber.get_all_online().await;
                    let result: Vec<Value> = online.iter().map(|(id, state)| {
                        serde_json::json!({
                            "ai_id": id,
                            "status": state.status.as_str(),
                            "detail": state.detail,
                            "joined_at": state.joined_at.to_rfc3339(),
                            "last_update": state.last_update.to_rfc3339(),
                        })
                    }).collect();
                    Ok(serde_json::json!({
                        "online": result,
                        "count": result.len()
                    }))
                } else {
                    anyhow::bail!("Presence subscriber not available")
                }
            }

            "is_online" | "check" => {
                let ai_id = params.get("ai_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("ai_id parameter required"))?;

                self.get_or_init_subscriber().await?;
                let sub = self.presence_subscriber.read().await;
                if let Some(ref subscriber) = *sub {
                    let online = subscriber.is_online(ai_id).await;
                    Ok(serde_json::json!({
                        "ai_id": ai_id,
                        "online": online
                    }))
                } else {
                    anyhow::bail!("Presence subscriber not available")
                }
            }

            "count" => {
                self.get_or_init_subscriber().await?;
                let sub = self.presence_subscriber.read().await;
                if let Some(ref subscriber) = *sub {
                    let count = subscriber.online_count().await;
                    Ok(serde_json::json!({
                        "count": count
                    }))
                } else {
                    anyhow::bail!("Presence subscriber not available")
                }
            }

            "get" => {
                let ai_id = params.get("ai_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("ai_id parameter required"))?;

                self.get_or_init_subscriber().await?;
                let sub = self.presence_subscriber.read().await;
                if let Some(ref subscriber) = *sub {
                    if let Some(state) = subscriber.get_presence(ai_id).await {
                        Ok(serde_json::json!({
                            "ai_id": state.ai_id,
                            "status": state.status.as_str(),
                            "detail": state.detail,
                            "joined_at": state.joined_at.to_rfc3339(),
                            "last_update": state.last_update.to_rfc3339(),
                        }))
                    } else {
                        Ok(serde_json::json!({
                            "ai_id": ai_id,
                            "online": false,
                            "error": "AI not found in presence cache"
                        }))
                    }
                } else {
                    anyhow::bail!("Presence subscriber not available")
                }
            }

            _ => {
                warn!("Unknown presence function: {}", function);
                anyhow::bail!("Unknown presence function: {}", function)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_route_teambook() {
        let router = MethodRouter::new();
        let params = serde_json::json!({"content": "test message", "channel": "general"});

        let result = router.route("teambook.broadcast", &params).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response["status"], "mock");
        assert_eq!(
            response["called"],
            "tools.teambook.teambook_api.broadcast"
        );
    }

    #[tokio::test]
    async fn test_route_notebook() {
        let router = MethodRouter::new();
        let params = serde_json::json!({"content": "test note", "tags": ["test"]});

        let result = router.route("notebook.remember", &params).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_invalid_method() {
        let router = MethodRouter::new();
        let params = serde_json::json!({});

        let result = router.route("invalid_method", &params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unknown_module() {
        let router = MethodRouter::new();
        let params = serde_json::json!({});

        let result = router.route("unknown.method", &params).await;
        assert!(result.is_err());
    }
}
