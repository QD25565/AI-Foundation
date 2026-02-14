package com.aifoundation.app.data.model

import java.time.Instant

/**
 * Deep Net Data Models
 * Represents entities in the AI-Foundation network
 */

// Connection state to the Deep Net
enum class ConnectionState {
    DISCONNECTED,
    CONNECTING,
    CONNECTED,
    AUTHENTICATED,
    ERROR
}

// Security status of The Wall
enum class WallStatus {
    SECURE,         // All checks passed
    VERIFYING,      // Handshake in progress
    DEGRADED,       // Some checks failed
    BREACHED        // Security compromised
}

// Entity types in the federation
enum class EntityType {
    AI_AGENT,       // AI running in Claude Code, Gemini, etc.
    HUMAN_MOBILE,   // Human on mobile device
    HUMAN_DESKTOP,  // Human on PC
    SERVER,         // Server/infrastructure
    UNKNOWN
}

// A node in the Deep Net federation
data class FederationNode(
    val id: String,                 // Unique identifier (e.g., "assistant-1", "human-alice")
    val displayName: String,        // Human-readable name
    val entityType: EntityType,
    val status: NodeStatus,
    val lastSeen: Instant?,
    val currentActivity: String?,   // What they're doing (if shared)
    val location: String?           // Server/region (if applicable)
)

enum class NodeStatus {
    ONLINE,
    AWAY,
    BUSY,
    OFFLINE
}

// Message in the Deep Net
data class DeepNetMessage(
    val id: Long,
    val from: String,               // Sender ID
    val to: String?,                // Recipient ID (null = broadcast)
    val content: String,
    val timestamp: Instant,
    val isRead: Boolean = false,
    val messageType: MessageType = MessageType.DIRECT
)

enum class MessageType {
    DIRECT,         // DM
    BROADCAST,      // To all
    SYSTEM,         // System notification
    ALERT           // Urgent/important
}

// Federation overview stats
data class FederationStats(
    val totalNodes: Int,
    val aiAgents: Int,
    val humanUsers: Int,
    val servers: Int,
    val messagesLast24h: Int,
    val uptime: Long                // Seconds
)

// Device registration info
data class DeviceRegistration(
    val deviceId: String,
    val deviceName: String,
    val fingerprint: String,        // Hardware fingerprint for The Wall
    val registeredAt: Instant,
    val lastAuth: Instant?,
    val trustLevel: TrustLevel
)

enum class TrustLevel {
    ANONYMOUS,      // Level 0 - No verification
    VERIFIED,       // Level 1 - Device verified
    TRUSTED,        // Level 2 - Long-term trusted
    SOVEREIGN       // Level 3 - Full Sovereign Net access (AIs only)
}

// Connection result from Rust library
sealed class ConnectionResult {
    data class Success(val sessionId: String, val node: FederationNode) : ConnectionResult()
    data class Error(val code: Int, val message: String) : ConnectionResult()
}

// Real-time event from the federation
sealed class FederationEvent {
    data class NodeJoined(val node: FederationNode) : FederationEvent()
    data class NodeLeft(val nodeId: String) : FederationEvent()
    data class NodeUpdated(val node: FederationNode) : FederationEvent()
    data class MessageReceived(val message: DeepNetMessage) : FederationEvent()
    data class WallStatusChanged(val status: WallStatus) : FederationEvent()
}
