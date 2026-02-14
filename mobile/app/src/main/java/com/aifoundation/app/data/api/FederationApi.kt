package com.aifoundation.app.data.api

import retrofit2.Response
import retrofit2.http.*

/**
 * Retrofit API interface for Federation Server
 * Connects to the HTTP bridge at port 31420
 */
interface FederationApi {

    // ============================================================================
    // REGISTRATION & STATUS
    // ============================================================================

    @POST("federation/register")
    suspend fun register(@Body request: RegisterRequest): Response<RegisterResponse>

    @POST("federation/reconnect")
    suspend fun reconnect(@Body request: ReconnectRequest): Response<ReconnectResponse>

    @GET("federation/status")
    suspend fun getStatus(): Response<StatusResponse>

    @GET("health")
    suspend fun healthCheck(): Response<HealthResponse>

    // ============================================================================
    // FEDERATION MEMBERS
    // ============================================================================

    @GET("federation/members")
    suspend fun getMembers(): Response<List<FederationMemberResponse>>

    @GET("federation/team")
    suspend fun getTeam(): Response<List<TeamMemberResponse>>

    // ============================================================================
    // MESSAGING
    // ============================================================================

    @POST("federation/dm")
    suspend fun sendDm(@Body request: DmRequest): Response<MessageResponse>

    @POST("federation/broadcast")
    suspend fun sendBroadcast(@Body request: BroadcastRequest): Response<MessageResponse>

    @GET("federation/messages")
    suspend fun getMessages(
        @Query("auth_token") authToken: String,
        @Query("limit") limit: Int = 20,
        @Query("message_type") messageType: String = ""
    ): Response<List<MessageItemResponse>>
}

// ============================================================================
// REQUEST TYPES
// ============================================================================

data class RegisterRequest(
    val device_name: String,
    val device_type: String,
    val fingerprint: String
)

data class ReconnectRequest(
    val device_id: String,
    val fingerprint: String
)

data class DmRequest(
    val auth_token: String,
    val to_ai: String,
    val content: String
)

data class BroadcastRequest(
    val auth_token: String,
    val content: String,
    val channel: String = "general"
)

// ============================================================================
// RESPONSE TYPES
// ============================================================================

data class RegisterResponse(
    val success: Boolean,
    val device_id: String,
    val auth_token: String,
    val error: String?
)

data class ReconnectResponse(
    val success: Boolean,
    val auth_token: String?,
    val error: String?
)

data class StatusResponse(
    val connected: Boolean,
    val server_uptime_secs: Long,
    val registered_devices: Int,
    val store_available: Boolean
)

data class HealthResponse(
    val status: String,
    val service: String,
    val version: String
)

data class FederationMemberResponse(
    val member_id: String,
    val member_type: String,
    val display_name: String,
    val status: String,
    val last_seen: String
)

data class TeamMemberResponse(
    val ai_id: String,
    val display_name: String,
    val status: String,
    val current_activity: String?,
    val last_seen: String
)

data class MessageResponse(
    val success: Boolean,
    val message_id: Long,
    val error: String?
)

data class MessageItemResponse(
    val id: Long,
    val from_id: String,
    val to_id: String?,
    val content: String,
    val timestamp: String,
    val message_type: String
)
