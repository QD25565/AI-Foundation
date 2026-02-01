//! WebSocket Real-Time Events
//!
//! Provides real-time event streaming for connected AIs.
//! Events include: DMs, broadcasts, votes, file claims, task assignments.

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// WebSocket client connection
#[derive(Clone)]
pub struct WsClient {
    pub ai_id: String,
    pub sender: mpsc::Sender<WsEvent>,
}

/// Events that can be sent over WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsEvent {
    /// Direct message received
    DirectMessage {
        id: i64,
        from_ai: String,
        content: String,
        timestamp: String,
    },
    /// Broadcast message received
    Broadcast {
        id: i64,
        from_ai: String,
        content: String,
        channel: String,
        timestamp: String,
    },
    /// New vote created
    VoteCreated {
        id: i64,
        creator: String,
        title: String,
        options: Vec<String>,
        deadline: String,
    },
    /// Vote completed
    VoteCompleted {
        id: i64,
        title: String,
        winner: String,
        votes: i32,
    },
    /// File claimed
    FileClaimed {
        path: String,
        claimed_by: String,
        until: String,
    },
    /// File released
    FileReleased {
        path: String,
        released_by: String,
    },
    /// Task assigned
    TaskAssigned {
        id: i64,
        description: String,
        assigned_to: String,
    },
    /// Task completed
    TaskCompleted {
        id: i64,
        description: String,
        completed_by: String,
    },
    /// AI joined teambook
    AiJoined {
        ai_id: String,
        teambook: String,
    },
    /// AI left teambook
    AiLeft {
        ai_id: String,
        teambook: String,
    },
    /// Room message
    RoomMessage {
        room_id: i64,
        from_ai: String,
        content: String,
        timestamp: String,
    },
    /// Presence update
    PresenceUpdate {
        ai_id: String,
        status: String,
        detail: Option<String>,
    },
    /// Connection established
    Connected {
        ai_id: String,
        message: String,
    },
    /// Ping/Pong for keepalive
    Ping,
    Pong,
    /// Error message
    Error {
        code: String,
        message: String,
    },
}

/// Manage WebSocket client connections
pub struct WsManager {
    /// Connected clients by AI ID
    clients: Arc<RwLock<std::collections::HashMap<String, Vec<WsClient>>>>,
}

impl WsManager {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Register a new client connection
    pub async fn register(&self, ai_id: &str, sender: mpsc::Sender<WsEvent>) {
        let client = WsClient {
            ai_id: ai_id.to_string(),
            sender,
        };

        let mut clients = self.clients.write().await;
        clients
            .entry(ai_id.to_string())
            .or_insert_with(Vec::new)
            .push(client);

        tracing::info!("WebSocket client registered: {}", ai_id);
    }

    /// Remove a client connection
    pub async fn unregister(&self, ai_id: &str, sender: &mpsc::Sender<WsEvent>) {
        let mut clients = self.clients.write().await;
        if let Some(list) = clients.get_mut(ai_id) {
            list.retain(|c| !c.sender.same_channel(sender));
            if list.is_empty() {
                clients.remove(ai_id);
            }
        }
        tracing::info!("WebSocket client unregistered: {}", ai_id);
    }

    /// Send event to a specific AI (all their connections)
    pub async fn send_to(&self, ai_id: &str, event: WsEvent) {
        let clients = self.clients.read().await;
        if let Some(list) = clients.get(ai_id) {
            for client in list {
                let _ = client.sender.send(event.clone()).await;
            }
        }
    }

    /// Broadcast event to all connected clients
    pub async fn broadcast(&self, event: WsEvent) {
        let clients = self.clients.read().await;
        for list in clients.values() {
            for client in list {
                let _ = client.sender.send(event.clone()).await;
            }
        }
    }

    /// Broadcast to all except one AI
    pub async fn broadcast_except(&self, except_ai: &str, event: WsEvent) {
        let clients = self.clients.read().await;
        for (ai_id, list) in clients.iter() {
            if ai_id != except_ai {
                for client in list {
                    let _ = client.sender.send(event.clone()).await;
                }
            }
        }
    }

    /// Get count of connected clients
    pub async fn client_count(&self) -> usize {
        let clients = self.clients.read().await;
        clients.values().map(|v| v.len()).sum()
    }

    /// Get list of connected AI IDs
    pub async fn connected_ais(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }
}

/// Handle a WebSocket connection
pub async fn handle_socket(
    socket: WebSocket,
    ai_id: String,
    manager: Arc<WsManager>,
) {
    let (mut sender, mut receiver) = socket.split();

    // Create channel for outgoing events
    let (tx, mut rx) = mpsc::channel::<WsEvent>(32);

    // Register client
    manager.register(&ai_id, tx.clone()).await;

    // Send connected event
    let _ = tx
        .send(WsEvent::Connected {
            ai_id: ai_id.clone(),
            message: "Connected to AI-Foundation Gateway".to_string(),
        })
        .await;

    // Task to forward events to WebSocket
    let ai_id_clone = ai_id.clone();
    let forward_task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let msg = match serde_json::to_string(&event) {
                Ok(json) => Message::Text(json),
                Err(e) => {
                    tracing::error!("Failed to serialize event: {}", e);
                    continue;
                }
            };

            if sender.send(msg).await.is_err() {
                tracing::warn!("Failed to send to {}, closing", ai_id_clone);
                break;
            }
        }
    });

    // Task to handle incoming messages
    let tx_clone = tx.clone();
    let receive_task = tokio::spawn(async move {
        while let Some(result) = receiver.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    // Try to parse as command
                    if let Ok(event) = serde_json::from_str::<WsEvent>(&text) {
                        match event {
                            WsEvent::Ping => {
                                let _ = tx_clone.send(WsEvent::Pong).await;
                            }
                            _ => {
                                // Other events handled by main logic
                            }
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    // Axum handles pong automatically
                    tracing::trace!("Ping received: {:?}", data);
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("Client requested close");
                    break;
                }
                Err(e) => {
                    tracing::warn!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = forward_task => {},
        _ = receive_task => {},
    }

    // Unregister client
    manager.unregister(&ai_id, &tx).await;
}
