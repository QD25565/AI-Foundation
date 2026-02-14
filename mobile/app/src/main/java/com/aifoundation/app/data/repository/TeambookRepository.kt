package com.aifoundation.app.data.repository

import com.aifoundation.app.data.api.*
import com.aifoundation.app.data.network.TeambookClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Repository for AI-Foundation teambook operations via HTTP API.
 * All methods call the CLI-backed REST endpoints.
 * Response data is CLI text output wrapped in ApiResponse envelope.
 */
class TeambookRepository {

    private val api: TeambookApi get() = TeambookClient.api

    // ============================================================================
    // PAIRING
    // ============================================================================

    suspend fun pairValidate(code: String): Result<PairValidateResponse> {
        return withContext(Dispatchers.IO) {
            try {
                val response = api.pairValidate(PairValidateRequest(code = code))
                if (response.isSuccessful && response.body()?.ok == true) {
                    Result.success(response.body()!!)
                } else {
                    val error = response.body()?.error ?: "Pairing failed"
                    Result.failure(Exception(error))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }

    // ============================================================================
    // STATUS
    // ============================================================================

    suspend fun getStatus(): Result<String> = callApi { api.getStatus() }

    // ============================================================================
    // MESSAGING
    // ============================================================================

    suspend fun getDms(limit: Int = 10): Result<String> = callApi { api.getDms(limit) }

    suspend fun sendDm(to: String, content: String): Result<String> =
        callApi { api.sendDm(SendDmBody(to = to, content = content)) }

    suspend fun getBroadcasts(limit: Int = 10): Result<String> = callApi { api.getBroadcasts(limit) }

    suspend fun sendBroadcast(content: String, channel: String = "general"): Result<String> =
        callApi { api.sendBroadcast(SendBroadcastBody(content = content, channel = channel)) }

    // ============================================================================
    // NOTEBOOK
    // ============================================================================

    suspend fun notebookRemember(content: String, tags: String? = null): Result<String> =
        callApi { api.notebookRemember(RememberBody(content = content, tags = tags)) }

    suspend fun notebookRecall(query: String, limit: Int = 10): Result<String> =
        callApi { api.notebookRecall(query, limit) }

    suspend fun notebookList(limit: Int = 10): Result<String> = callApi { api.notebookList(limit) }

    suspend fun notebookGet(id: String): Result<String> = callApi { api.notebookGet(id) }

    suspend fun notebookDelete(id: String): Result<String> = callApi { api.notebookDelete(id) }

    // ============================================================================
    // TASKS
    // ============================================================================

    suspend fun getTasks(filter: String = "all", limit: Int = 20): Result<String> =
        callApi { api.getTasks(filter, limit) }

    suspend fun createTask(description: String, tasks: String? = null): Result<String> =
        callApi { api.createTask(CreateTaskBody(description = description, tasks = tasks)) }

    suspend fun getTask(id: String): Result<String> = callApi { api.getTask(id) }

    suspend fun updateTask(id: String, status: String, reason: String? = null): Result<String> =
        callApi { api.updateTask(id, UpdateTaskBody(status = status, reason = reason)) }

    // ============================================================================
    // DIALOGUES
    // ============================================================================

    suspend fun getDialogues(limit: Int = 10): Result<String> = callApi { api.getDialogues(limit) }

    suspend fun startDialogue(responder: String, topic: String): Result<String> =
        callApi { api.startDialogue(StartDialogueBody(responder = responder, topic = topic)) }

    suspend fun getDialogue(id: String): Result<String> = callApi { api.getDialogue(id) }

    suspend fun respondDialogue(id: String, response: String): Result<String> =
        callApi { api.respondDialogue(id, RespondDialogueBody(response = response)) }

    // ============================================================================
    // HELPER
    // ============================================================================

    /**
     * Generic API call handler. Unwraps ApiResponse envelope.
     * Returns the `data` field on success, or an error.
     */
    private suspend fun callApi(
        call: suspend () -> retrofit2.Response<ApiResponse>
    ): Result<String> {
        return withContext(Dispatchers.IO) {
            try {
                val response = call()
                val body = response.body()
                if (response.isSuccessful && body?.ok == true) {
                    Result.success(body.data ?: "")
                } else {
                    val error = body?.error ?: "Request failed (${response.code()})"
                    Result.failure(Exception(error))
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }
    }
}
