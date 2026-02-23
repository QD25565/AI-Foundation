package com.aifoundation.app.data.api

import com.aifoundation.app.data.model.*
import retrofit2.Response
import retrofit2.http.*

/**
 * Retrofit interface for ai-foundation-mobile-api (port 8081).
 *
 * All authenticated endpoints expect:  Authorization: Bearer <token>
 * (added automatically by the auth interceptor in TeambookClient)
 */
interface TeambookApi {

    // ── Pairing (no auth) ────────────────────────────────────────────────────

    @POST("api/pair/request")
    suspend fun pairRequest(@Body request: PairRequestBody): Response<PairRequestResponse>

    @POST("api/pair/validate")
    suspend fun pairValidate(@Body request: PairValidateBody): Response<PairValidateResponse>

    // ── Status (no auth) ─────────────────────────────────────────────────────

    @GET("api/status")
    suspend fun getStatus(): Response<TeamStatusResponse>

    // ── Team (auth required) ─────────────────────────────────────────────────

    @GET("api/team")
    suspend fun getTeam(): Response<TeamListResponse>

    // ── DMs (auth required) ──────────────────────────────────────────────────

    @GET("api/dms")
    suspend fun getDms(@Query("limit") limit: Int = 20): Response<DmsResponse>

    @POST("api/dms")
    suspend fun sendDm(@Body request: SendDmBody): Response<ActionResponse>

    // ── Broadcasts (auth required) ────────────────────────────────────────────

    @GET("api/broadcasts")
    suspend fun getBroadcasts(@Query("limit") limit: Int = 20): Response<BroadcastsResponse>

    @POST("api/broadcasts")
    suspend fun sendBroadcast(@Body request: SendBroadcastBody): Response<ActionResponse>

    // ── Tasks (auth required) ─────────────────────────────────────────────────

    @GET("api/tasks")
    suspend fun getTasks(): Response<TasksResponse>

    @POST("api/tasks")
    suspend fun createTask(@Body request: CreateTaskBody): Response<ActionResponse>

    @PATCH("api/tasks/{id}")
    suspend fun updateTask(
        @Path("id") id: String,
        @Body request: UpdateTaskBody
    ): Response<ActionResponse>

    // ── Dialogues (auth required) ─────────────────────────────────────────────

    @GET("api/dialogues")
    suspend fun getDialogues(): Response<DialoguesResponse>

    @POST("api/dialogues")
    suspend fun startDialogue(@Body request: StartDialogueBody): Response<ActionResponse>

    @POST("api/dialogues/{id}/respond")
    suspend fun respondDialogue(
        @Path("id") id: String,
        @Body request: RespondDialogueBody
    ): Response<ActionResponse>

    // ── Notebook (auth required) ──────────────────────────────────────────────

    @GET("api/notebook")
    suspend fun getNotes(@Query("limit") limit: Int = 20): Response<NotesResponse>

    @POST("api/notebook/remember")
    suspend fun rememberNote(@Body request: RememberBody): Response<ActionResponse>

    @GET("api/notebook/recall")
    suspend fun recallNotes(@Query("q") query: String): Response<NoteSearchResponse>

    // ── Unpair (auth required) ────────────────────────────────────────────────

    @POST("api/unpair")
    suspend fun unpair(): Response<ActionResponse>
}

// ── Pairing ───────────────────────────────────────────────────────────────────

data class PairRequestBody(val h_id: String = "")

data class PairRequestResponse(
    val ok: Boolean,
    val code: String?,
    val h_id: String?,
    val expires_in_secs: Long?,
    val error: String?
)

data class PairValidateBody(val code: String)

data class PairValidateResponse(
    val ok: Boolean,
    val h_id: String?,
    val token: String?,
    val pending: Boolean?,   // true = code exists but not yet approved
    val error: String?
)

// ── Typed response wrappers ───────────────────────────────────────────────────
// Using concrete types (not generics) so Gson can deserialize List<T> correctly.

data class TeamStatusResponse(
    val ok: Boolean,
    val online_count: Int?,
    val members: List<TeamMember>?,
    val error: String?
)

data class TeamListResponse(
    val ok: Boolean,
    val online_count: Int? = null,
    val members: List<TeamMember>?,   // server returns "members" not "data"
    val error: String?
)

data class DmsResponse(
    val ok: Boolean,
    val data: List<Dm>?,
    val error: String?
)

data class BroadcastsResponse(
    val ok: Boolean,
    val data: List<Broadcast>?,
    val error: String?
)

data class TasksResponse(
    val ok: Boolean,
    val data: List<Task>?,
    val error: String?
)

data class DialoguesResponse(
    val ok: Boolean,
    val data: List<Dialogue>?,
    val error: String?
)

data class NotesResponse(
    val ok: Boolean,
    val data: List<Note>?,
    val error: String?
)

data class NoteSearchResponse(
    val ok: Boolean,
    val data: List<NoteSearchResult>?,
    val error: String?
)

data class ActionResponse(val ok: Boolean, val error: String?)

// ── Request bodies ────────────────────────────────────────────────────────────

data class SendDmBody(val to: String, val content: String)
data class SendBroadcastBody(val content: String, val channel: String? = null)
data class CreateTaskBody(val description: String)
data class UpdateTaskBody(val status: String, val reason: String? = null)
data class StartDialogueBody(val responder: String, val topic: String)
data class RespondDialogueBody(val response: String)
data class RememberBody(val content: String, val tags: String? = null)
