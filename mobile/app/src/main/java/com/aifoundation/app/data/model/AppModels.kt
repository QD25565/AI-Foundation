package com.aifoundation.app.data.model

/**
 * Typed data models for the AI-Foundation mobile app.
 *
 * All types mirror the JSON structs returned by ai-foundation-mobile-api
 * (parser.rs in the backend). Field names use snake_case to match the JSON
 * without needing @SerializedName annotations everywhere — Gson handles it.
 */

// ── Team ──────────────────────────────────────────────────────────────────────

data class TeamMember(
    val ai_id: String,
    val type: String,          // "ai" | "human"
    val online: Boolean,
    val last_seen: String,
    val activity: String?
) {
    val isAi: Boolean get() = type == "ai"
    val displayName: String get() = ai_id
}

// ── Messages ──────────────────────────────────────────────────────────────────

data class Dm(
    val id: Long,
    val from: String,
    val to: String,
    val content: String,
    val timestamp: String
)

data class Broadcast(
    val id: Long,
    val from: String,
    val content: String,
    val timestamp: String,
    val channel: String
)

// ── Tasks ─────────────────────────────────────────────────────────────────────

data class Task(
    val id: String,
    val description: String,
    val status: String,        // "pending" | "in_progress" | "completed" | "blocked" | "done"
    val owner: String?,
    val created_at: String
)

// ── Dialogues ─────────────────────────────────────────────────────────────────

data class Dialogue(
    val id: Long,
    val topic: String,
    val initiator: String,
    val responder: String,
    val status: String,
    val message_count: Int,
    val last_activity: String
)

// ── Notebook ──────────────────────────────────────────────────────────────────

data class Note(
    val id: Long,
    val content: String,
    val tags: List<String>,
    val pinned: Boolean,
    val created_at: String
)

data class NoteSearchResult(
    val id: Long,
    val content: String,
    val tags: List<String>,
    val score: Float
)

// ── SSE live events ───────────────────────────────────────────────────────────

sealed class LiveEvent {
    data class DmReceived(val dm: Dm) : LiveEvent()
    data class BroadcastReceived(val bc: Broadcast) : LiveEvent()
    data class TeamUpdated(val members: List<TeamMember>) : LiveEvent()
    data class TaskUpdated(val task: Task) : LiveEvent()
}
