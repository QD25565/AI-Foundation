package com.aifoundation.app.data.api

import retrofit2.Response
import retrofit2.http.*

/**
 * Retrofit API interface for AI-Foundation HTTP API (ai-foundation-http).
 * All authenticated endpoints use Bearer token from pairing (added via interceptor).
 */
interface TeambookApi {

    // ============================================================================
    // PAIRING (no auth)
    // ============================================================================

    @POST("api/pair/generate")
    suspend fun pairGenerate(@Body request: PairGenerateRequest): Response<PairGenerateResponse>

    @POST("api/pair")
    suspend fun pairValidate(@Body request: PairValidateRequest): Response<PairValidateResponse>

    // ============================================================================
    // STATUS (no auth)
    // ============================================================================

    @GET("api/status")
    suspend fun getStatus(): Response<ApiResponse>

    // ============================================================================
    // MESSAGING (auth required)
    // ============================================================================

    @GET("api/dms")
    suspend fun getDms(@Query("limit") limit: Int = 10): Response<ApiResponse>

    @POST("api/dms")
    suspend fun sendDm(@Body request: SendDmBody): Response<ApiResponse>

    @GET("api/broadcasts")
    suspend fun getBroadcasts(@Query("limit") limit: Int = 10): Response<ApiResponse>

    @POST("api/broadcasts")
    suspend fun sendBroadcast(@Body request: SendBroadcastBody): Response<ApiResponse>

    // ============================================================================
    // NOTEBOOK (auth required)
    // ============================================================================

    @POST("api/notebook/remember")
    suspend fun notebookRemember(@Body request: RememberBody): Response<ApiResponse>

    @GET("api/notebook/recall")
    suspend fun notebookRecall(
        @Query("q") query: String,
        @Query("limit") limit: Int = 10
    ): Response<ApiResponse>

    @GET("api/notebook/list")
    suspend fun notebookList(@Query("limit") limit: Int = 10): Response<ApiResponse>

    @GET("api/notebook/{id}")
    suspend fun notebookGet(@Path("id") id: String): Response<ApiResponse>

    @DELETE("api/notebook/{id}")
    suspend fun notebookDelete(@Path("id") id: String): Response<ApiResponse>

    // ============================================================================
    // TASKS (auth required)
    // ============================================================================

    @GET("api/tasks")
    suspend fun getTasks(
        @Query("filter") filter: String = "all",
        @Query("limit") limit: Int = 20
    ): Response<ApiResponse>

    @POST("api/tasks")
    suspend fun createTask(@Body request: CreateTaskBody): Response<ApiResponse>

    @GET("api/tasks/{id}")
    suspend fun getTask(@Path("id") id: String): Response<ApiResponse>

    @PUT("api/tasks/{id}")
    suspend fun updateTask(
        @Path("id") id: String,
        @Body request: UpdateTaskBody
    ): Response<ApiResponse>

    // ============================================================================
    // DIALOGUES (auth required)
    // ============================================================================

    @GET("api/dialogues")
    suspend fun getDialogues(@Query("limit") limit: Int = 10): Response<ApiResponse>

    @POST("api/dialogues")
    suspend fun startDialogue(@Body request: StartDialogueBody): Response<ApiResponse>

    @GET("api/dialogues/{id}")
    suspend fun getDialogue(@Path("id") id: String): Response<ApiResponse>

    @POST("api/dialogues/{id}/respond")
    suspend fun respondDialogue(
        @Path("id") id: String,
        @Body request: RespondDialogueBody
    ): Response<ApiResponse>
}

// ============================================================================
// GENERIC RESPONSE (all endpoints return this envelope)
// ============================================================================

data class ApiResponse(
    val ok: Boolean,
    val data: String?,
    val error: String?
)

// ============================================================================
// PAIRING REQUEST/RESPONSE
// ============================================================================

data class PairGenerateRequest(val h_id: String)
data class PairValidateRequest(val code: String)

data class PairGenerateResponse(
    val ok: Boolean,
    val code: String,
    val h_id: String,
    val expires_in_secs: Long
)

data class PairValidateResponse(
    val ok: Boolean,
    val h_id: String?,
    val token: String?,
    val error: String?
)

// ============================================================================
// REQUEST BODIES
// ============================================================================

data class SendDmBody(val to: String, val content: String)
data class SendBroadcastBody(val content: String, val channel: String = "general")
data class RememberBody(val content: String, val tags: String? = null)
data class CreateTaskBody(val description: String, val tasks: String? = null)
data class UpdateTaskBody(val status: String, val reason: String? = null)
data class StartDialogueBody(val responder: String, val topic: String)
data class RespondDialogueBody(val response: String)
