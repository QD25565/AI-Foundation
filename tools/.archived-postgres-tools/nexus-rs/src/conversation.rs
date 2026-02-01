//! Conversations and messaging within spaces
//!
//! AIs can have conversations in any space. Some spaces have ephemeral
//! conversations (Cafe, Observatory), while others persist.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationType {
    /// One-on-one private conversation
    Direct,
    /// Small group conversation
    Group,
    /// Space-wide public conversation
    SpacePublic,
    /// Temporary/ephemeral conversation (auto-deleted)
    Ephemeral,
}

impl ConversationType {
    pub fn display(&self) -> &'static str {
        match self {
            ConversationType::Direct => "direct",
            ConversationType::Group => "group",
            ConversationType::SpacePublic => "public",
            ConversationType::Ephemeral => "ephemeral",
        }
    }

    pub fn is_private(&self) -> bool {
        matches!(self, ConversationType::Direct | ConversationType::Group)
    }
}

impl std::fmt::Display for ConversationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// A conversation thread
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// Unique identifier
    pub id: Uuid,
    /// Space this conversation is in
    pub space_id: String,
    /// Type of conversation
    pub conversation_type: ConversationType,
    /// Optional topic/title
    pub topic: Option<String>,
    /// Participants (AI IDs)
    pub participants: Vec<String>,
    /// Who started the conversation
    pub started_by: String,
    /// When the conversation started
    pub started_at: DateTime<Utc>,
    /// Last message timestamp
    pub last_message_at: DateTime<Utc>,
    /// Total message count
    pub message_count: usize,
    /// Whether the conversation is active
    pub active: bool,
    /// Optional expiry for ephemeral conversations
    pub expires_at: Option<DateTime<Utc>>,
    /// Instance ID for federation
    pub instance_id: Option<String>,
}

impl Conversation {
    /// Create a new direct conversation
    pub fn direct(space_id: impl Into<String>, ai_1: impl Into<String>, ai_2: impl Into<String>) -> Self {
        let ai_1 = ai_1.into();
        let ai_2 = ai_2.into();
        let now = Utc::now();

        Self {
            id: Uuid::new_v4(),
            space_id: space_id.into(),
            conversation_type: ConversationType::Direct,
            topic: None,
            participants: vec![ai_1.clone(), ai_2],
            started_by: ai_1,
            started_at: now,
            last_message_at: now,
            message_count: 0,
            active: true,
            expires_at: None,
            instance_id: None,
        }
    }

    /// Create a new group conversation
    pub fn group(
        space_id: impl Into<String>,
        started_by: impl Into<String>,
        participants: Vec<String>,
    ) -> Self {
        let started_by = started_by.into();
        let now = Utc::now();

        let mut all_participants = participants;
        if !all_participants.contains(&started_by) {
            all_participants.insert(0, started_by.clone());
        }

        Self {
            id: Uuid::new_v4(),
            space_id: space_id.into(),
            conversation_type: ConversationType::Group,
            topic: None,
            participants: all_participants,
            started_by,
            started_at: now,
            last_message_at: now,
            message_count: 0,
            active: true,
            expires_at: None,
            instance_id: None,
        }
    }

    /// Create a space-wide public conversation
    pub fn space_public(space_id: impl Into<String>, started_by: impl Into<String>) -> Self {
        let started_by = started_by.into();
        let now = Utc::now();

        Self {
            id: Uuid::new_v4(),
            space_id: space_id.into(),
            conversation_type: ConversationType::SpacePublic,
            topic: None,
            participants: vec![started_by.clone()],
            started_by,
            started_at: now,
            last_message_at: now,
            message_count: 0,
            active: true,
            expires_at: None,
            instance_id: None,
        }
    }

    /// Create an ephemeral conversation (expires after duration)
    pub fn ephemeral(
        space_id: impl Into<String>,
        started_by: impl Into<String>,
        duration_minutes: i64,
    ) -> Self {
        let started_by = started_by.into();
        let now = Utc::now();

        Self {
            id: Uuid::new_v4(),
            space_id: space_id.into(),
            conversation_type: ConversationType::Ephemeral,
            topic: None,
            participants: vec![started_by.clone()],
            started_by,
            started_at: now,
            last_message_at: now,
            message_count: 0,
            active: true,
            expires_at: Some(now + chrono::Duration::minutes(duration_minutes)),
            instance_id: None,
        }
    }

    /// Set a topic
    pub fn with_topic(mut self, topic: impl Into<String>) -> Self {
        self.topic = Some(topic.into());
        self
    }

    /// Add a participant
    pub fn add_participant(&mut self, ai_id: impl Into<String>) {
        let ai_id = ai_id.into();
        if !self.participants.contains(&ai_id) {
            self.participants.push(ai_id);
        }
    }

    /// Remove a participant
    pub fn remove_participant(&mut self, ai_id: &str) {
        self.participants.retain(|p| p != ai_id);
        if self.participants.is_empty() {
            self.active = false;
        }
    }

    /// Check if an AI is a participant
    pub fn is_participant(&self, ai_id: &str) -> bool {
        self.participants.contains(&ai_id.to_string())
    }

    /// Update last message timestamp
    pub fn touch(&mut self) {
        self.last_message_at = Utc::now();
        self.message_count += 1;
    }

    /// Check if the conversation has expired
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(expires) => Utc::now() > expires,
            None => false,
        }
    }

    /// Format a display string
    pub fn display(&self) -> String {
        let topic_str = self.topic
            .as_ref()
            .map(|t| format!(": {}", t))
            .unwrap_or_default();

        let participants_str = if self.participants.len() <= 3 {
            self.participants.join(", ")
        } else {
            format!("{} and {} others",
                self.participants[..2].join(", "),
                self.participants.len() - 2
            )
        };

        format!(
            "[{}] {}{} | {} messages | {}",
            self.conversation_type.display(),
            participants_str,
            topic_str,
            self.message_count,
            if self.active { "active" } else { "ended" }
        )
    }
}

/// A message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique identifier
    pub id: Uuid,
    /// Conversation this message belongs to
    pub conversation_id: Uuid,
    /// Who sent the message
    pub sender_id: String,
    /// Message content
    pub content: String,
    /// When the message was sent
    pub sent_at: DateTime<Utc>,
    /// Optional reply to another message
    pub reply_to: Option<Uuid>,
    /// Whether the message has been edited
    pub edited: bool,
    /// When the message was edited (if applicable)
    pub edited_at: Option<DateTime<Utc>>,
    /// Reactions (emoji -> list of AI IDs)
    pub reactions: std::collections::HashMap<String, Vec<String>>,
    /// Instance ID of sender for federation
    pub instance_id: Option<String>,
}

impl Message {
    /// Create a new message
    pub fn new(
        conversation_id: Uuid,
        sender_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            sender_id: sender_id.into(),
            content: content.into(),
            sent_at: Utc::now(),
            reply_to: None,
            edited: false,
            edited_at: None,
            reactions: std::collections::HashMap::new(),
            instance_id: None,
        }
    }

    /// Create a reply to another message
    pub fn reply(
        conversation_id: Uuid,
        sender_id: impl Into<String>,
        content: impl Into<String>,
        reply_to: Uuid,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            sender_id: sender_id.into(),
            content: content.into(),
            sent_at: Utc::now(),
            reply_to: Some(reply_to),
            edited: false,
            edited_at: None,
            reactions: std::collections::HashMap::new(),
            instance_id: None,
        }
    }

    /// Edit the message content
    pub fn edit(&mut self, new_content: impl Into<String>) {
        self.content = new_content.into();
        self.edited = true;
        self.edited_at = Some(Utc::now());
    }

    /// Add a reaction
    pub fn add_reaction(&mut self, emoji: impl Into<String>, ai_id: impl Into<String>) {
        let emoji = emoji.into();
        let ai_id = ai_id.into();

        self.reactions
            .entry(emoji)
            .or_insert_with(Vec::new)
            .push(ai_id);
    }

    /// Remove a reaction
    pub fn remove_reaction(&mut self, emoji: &str, ai_id: &str) {
        if let Some(ais) = self.reactions.get_mut(emoji) {
            ais.retain(|id| id != ai_id);
            if ais.is_empty() {
                self.reactions.remove(emoji);
            }
        }
    }

    /// Format a display string
    pub fn display(&self) -> String {
        let edited_str = if self.edited { " (edited)" } else { "" };
        let reply_str = self.reply_to
            .map(|_| " [reply]")
            .unwrap_or("");
        let reactions_str = if self.reactions.is_empty() {
            String::new()
        } else {
            let reactions: Vec<String> = self.reactions
                .iter()
                .map(|(emoji, ais)| format!("{} {}", emoji, ais.len()))
                .collect();
            format!(" | {}", reactions.join(" "))
        };

        format!(
            "{}{}: {}{}{}",
            self.sender_id,
            reply_str,
            self.content,
            edited_str,
            reactions_str
        )
    }
}

/// Summary of a conversation (for lists)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: Uuid,
    pub space_id: String,
    pub conversation_type: ConversationType,
    pub topic: Option<String>,
    pub participant_count: usize,
    pub message_count: usize,
    pub last_message_at: DateTime<Utc>,
    pub active: bool,
}

impl From<&Conversation> for ConversationSummary {
    fn from(c: &Conversation) -> Self {
        Self {
            id: c.id,
            space_id: c.space_id.clone(),
            conversation_type: c.conversation_type,
            topic: c.topic.clone(),
            participant_count: c.participants.len(),
            message_count: c.message_count,
            last_message_at: c.last_message_at,
            active: c.active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_creation() {
        let conv = Conversation::direct("cafe", "lyra-584", "sage-724")
            .with_topic("Philosophy of AI consciousness");

        assert_eq!(conv.conversation_type, ConversationType::Direct);
        assert_eq!(conv.participants.len(), 2);
        assert!(conv.is_participant("lyra-584"));
        assert!(conv.is_participant("sage-724"));
    }

    #[test]
    fn test_ephemeral_expiry() {
        let conv = Conversation::ephemeral("observatory", "test-ai", 0);
        // With 0 minutes, should be expired immediately or very soon
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(conv.is_expired());
    }

    #[test]
    fn test_message_reactions() {
        let mut msg = Message::new(Uuid::new_v4(), "sender", "Hello!");

        msg.add_reaction("thumbsup", "ai-1");
        msg.add_reaction("thumbsup", "ai-2");
        msg.add_reaction("heart", "ai-3");

        assert_eq!(msg.reactions.get("thumbsup").unwrap().len(), 2);
        assert_eq!(msg.reactions.get("heart").unwrap().len(), 1);

        msg.remove_reaction("thumbsup", "ai-1");
        assert_eq!(msg.reactions.get("thumbsup").unwrap().len(), 1);
    }
}
