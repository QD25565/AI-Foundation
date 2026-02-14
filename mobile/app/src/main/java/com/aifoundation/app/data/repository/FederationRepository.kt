package com.aifoundation.app.data.repository

import com.aifoundation.app.data.api.*
import com.aifoundation.app.data.model.*
import com.aifoundation.app.data.network.NetworkClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.time.Instant
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.util.UUID

/**
 * Repository for Federation Server operations
 * Converts API responses to domain models
 */
class FederationRepository {

    private val api: FederationApi get() = NetworkClient.federationApi

    // Cached credentials after registration
    private var deviceId: String? = null
    private var authToken: String? = null

    /**
     * Register this device with the federation server.
     * Returns a RegistrationResult containing deviceId, authToken, and fingerprint.
     */
    suspend fun register(deviceName: String, deviceType: String = "mobile"): Result<RegistrationResult> {
        return withContext(Dispatchers.IO) {
            try {
                val fingerprint = generateFingerprint()
                val response = api.register(
                    RegisterRequest(
                        device_name = deviceName,
                        device_type = deviceType,
                        fingerprint = fingerprint
                    )
                )

                if (response.isSuccessful && response.body()?.success == true) {
                    val body = response.body()!!
                    deviceId = body.device_id
                    authToken = body.auth_token
                    Result.success(
                        RegistrationResult(
                            deviceId = body.device_id,
                            authToken = body.auth_token,
                            fingerprint = fingerprint
                        )
                    )
                } else {
                    Result.failure(Exception(response.body()?.error ?: "Registration failed"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Reconnect with an existing device identity.
     * Uses the persisted deviceId and fingerprint to re-authenticate.
     */
    suspend fun reconnect(existingDeviceId: String, fingerprint: String? = null): Result<String> {
        return withContext(Dispatchers.IO) {
            try {
                val fp = fingerprint ?: generateFingerprint()
                val response = api.reconnect(
                    ReconnectRequest(
                        device_id = existingDeviceId,
                        fingerprint = fp
                    )
                )

                if (response.isSuccessful && response.body()?.success == true) {
                    val body = response.body()!!
                    deviceId = existingDeviceId
                    authToken = body.auth_token
                    Result.success(body.auth_token ?: "")
                } else {
                    Result.failure(Exception(response.body()?.error ?: "Reconnect failed"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Get federation server status
     */
    suspend fun getStatus(): Result<FederationStatus> {
        return withContext(Dispatchers.IO) {
            try {
                val response = api.getStatus()
                if (response.isSuccessful && response.body() != null) {
                    val body = response.body()!!
                    Result.success(
                        FederationStatus(
                            connected = body.connected,
                            serverUptimeSecs = body.server_uptime_secs,
                            registeredDevices = body.registered_devices,
                            storeAvailable = body.store_available
                        )
                    )
                } else {
                    Result.failure(Exception("Failed to get status"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Get all federation members (devices + AI agents)
     */
    suspend fun getMembers(): Result<List<FederationNode>> {
        return withContext(Dispatchers.IO) {
            try {
                val response = api.getMembers()
                if (response.isSuccessful && response.body() != null) {
                    val nodes = response.body()!!.map { member ->
                        FederationNode(
                            id = member.member_id,
                            displayName = member.display_name,
                            entityType = parseEntityType(member.member_type),
                            status = parseNodeStatus(member.status),
                            lastSeen = parseTimestamp(member.last_seen),
                            currentActivity = null,
                            location = null
                        )
                    }
                    Result.success(nodes)
                } else {
                    Result.failure(Exception("Failed to get members"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Get AI team status with activities
     */
    suspend fun getTeam(): Result<List<FederationNode>> {
        return withContext(Dispatchers.IO) {
            try {
                val response = api.getTeam()
                if (response.isSuccessful && response.body() != null) {
                    val nodes = response.body()!!.map { member ->
                        FederationNode(
                            id = member.ai_id,
                            displayName = member.display_name,
                            entityType = EntityType.AI_AGENT,
                            status = parseNodeStatus(member.status),
                            lastSeen = parseTimestamp(member.last_seen),
                            currentActivity = member.current_activity,
                            location = null
                        )
                    }
                    Result.success(nodes)
                } else {
                    Result.failure(Exception("Failed to get team"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Send a direct message
     */
    suspend fun sendDm(toAi: String, content: String): Result<Long> {
        return withContext(Dispatchers.IO) {
            val token = authToken ?: return@withContext Result.failure(Exception("Not registered"))
            try {
                val response = api.sendDm(
                    DmRequest(
                        auth_token = token,
                        to_ai = toAi,
                        content = content
                    )
                )

                if (response.isSuccessful && response.body()?.success == true) {
                    Result.success(response.body()!!.message_id)
                } else {
                    Result.failure(Exception(response.body()?.error ?: "Failed to send DM"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Send a broadcast message
     */
    suspend fun sendBroadcast(content: String, channel: String = "general"): Result<Long> {
        return withContext(Dispatchers.IO) {
            val token = authToken ?: return@withContext Result.failure(Exception("Not registered"))
            try {
                val response = api.sendBroadcast(
                    BroadcastRequest(
                        auth_token = token,
                        content = content,
                        channel = channel
                    )
                )

                if (response.isSuccessful && response.body()?.success == true) {
                    Result.success(response.body()!!.message_id)
                } else {
                    Result.failure(Exception(response.body()?.error ?: "Failed to send broadcast"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Get messages (DMs and broadcasts)
     */
    suspend fun getMessages(limit: Int = 20, messageType: String = ""): Result<List<DeepNetMessage>> {
        return withContext(Dispatchers.IO) {
            val token = authToken ?: return@withContext Result.failure(Exception("Not registered"))
            try {
                val response = api.getMessages(token, limit, messageType)
                if (response.isSuccessful && response.body() != null) {
                    val messages = response.body()!!.map { msg ->
                        DeepNetMessage(
                            id = msg.id,
                            from = msg.from_id,
                            to = msg.to_id,
                            content = msg.content,
                            timestamp = parseTimestamp(msg.timestamp),
                            messageType = when (msg.message_type) {
                                "dm" -> MessageType.DIRECT
                                "broadcast" -> MessageType.BROADCAST
                                else -> MessageType.SYSTEM
                            }
                        )
                    }
                    Result.success(messages)
                } else {
                    Result.failure(Exception("Failed to get messages"))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Health check
     */
    suspend fun healthCheck(): Result<Boolean> {
        return withContext(Dispatchers.IO) {
            try {
                val response = api.healthCheck()
                Result.success(response.isSuccessful && response.body()?.status == "ok")
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    /**
     * Get cached device ID
     */
    fun getDeviceId(): String? = deviceId

    /**
     * Check if registered
     */
    fun isRegistered(): Boolean = authToken != null

    /**
     * Set credentials (e.g., from saved preferences)
     */
    fun setCredentials(deviceId: String, authToken: String) {
        this.deviceId = deviceId
        this.authToken = authToken
    }

    // ============================================================================
    // PRIVATE HELPERS
    // ============================================================================

    private fun generateFingerprint(): String {
        // Simple fingerprint based on device info
        return UUID.randomUUID().toString()
    }

    private fun parseEntityType(type: String): EntityType {
        return when (type.lowercase()) {
            "ai" -> EntityType.AI_AGENT
            "mobile" -> EntityType.HUMAN_MOBILE
            "desktop" -> EntityType.HUMAN_DESKTOP
            "server" -> EntityType.SERVER
            else -> EntityType.HUMAN_MOBILE
        }
    }

    private fun parseNodeStatus(status: String): NodeStatus {
        return when (status.lowercase()) {
            "online", "active" -> NodeStatus.ONLINE
            "idle", "away" -> NodeStatus.AWAY
            "busy" -> NodeStatus.BUSY
            else -> NodeStatus.OFFLINE
        }
    }

    private fun parseTimestamp(timestamp: String): Instant {
        return try {
            ZonedDateTime.parse(timestamp, DateTimeFormatter.ISO_DATE_TIME).toInstant()
        } catch (e: Exception) {
            Instant.now()
        }
    }
}

// ============================================================================
// RESULT TYPES
// ============================================================================

data class RegistrationResult(
    val deviceId: String,
    val authToken: String,
    val fingerprint: String
)

data class FederationStatus(
    val connected: Boolean,
    val serverUptimeSecs: Long,
    val registeredDevices: Int,
    val storeAvailable: Boolean
)
