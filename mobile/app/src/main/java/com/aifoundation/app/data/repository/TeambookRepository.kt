package com.aifoundation.app.data.repository

import com.aifoundation.app.data.api.*
import com.aifoundation.app.data.model.*
import com.aifoundation.app.data.network.TeambookClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Repository for AI-Foundation mobile-api operations.
 * All methods return typed Result<T> — no raw strings.
 *
 * Error handling: exceptions are caught and wrapped as Result.failure.
 * HTTP errors (non-2xx or ok:false) are converted to descriptive exceptions.
 */
class TeambookRepository {

    private val api: TeambookApi get() = TeambookClient.api

    // ── Pairing ───────────────────────────────────────────────────────────────

    suspend fun pairRequest(hId: String = ""): Result<PairRequestResponse> =
        withContext(Dispatchers.IO) {
            runCatching {
                val resp = api.pairRequest(PairRequestBody(h_id = hId))
                val body = resp.body() ?: error("Empty response (${resp.code()})")
                if (!body.ok) error(body.error ?: "Pair request failed")
                body
            }
        }

    suspend fun pairValidate(code: String): Result<PairValidateResponse> =
        withContext(Dispatchers.IO) {
            runCatching {
                val resp = api.pairValidate(PairValidateBody(code = code))
                val body = resp.body() ?: error("Empty response (${resp.code()})")
                body // caller inspects ok / pending / token
            }
        }

    // ── Status ────────────────────────────────────────────────────────────────

    suspend fun getStatus(): Result<TeamStatusResponse> =
        withContext(Dispatchers.IO) {
            runCatching {
                val resp = api.getStatus()
                val body = resp.body() ?: error("Empty response (${resp.code()})")
                if (!body.ok) error(body.error ?: "Status failed")
                body
            }
        }

    // ── Team ──────────────────────────────────────────────────────────────────

    suspend fun getTeam(): Result<List<TeamMember>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.getTeam().body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "getTeam failed")
                body.members ?: emptyList()
            }
        }

    // ── DMs ───────────────────────────────────────────────────────────────────

    suspend fun getDms(limit: Int = 20): Result<List<Dm>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.getDms(limit).body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "getDms failed")
                body.data ?: emptyList()
            }
        }

    suspend fun sendDm(to: String, content: String): Result<Unit> =
        action { api.sendDm(SendDmBody(to = to, content = content)) }

    // ── Broadcasts ────────────────────────────────────────────────────────────

    suspend fun getBroadcasts(limit: Int = 20): Result<List<Broadcast>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.getBroadcasts(limit).body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "getBroadcasts failed")
                body.data ?: emptyList()
            }
        }

    suspend fun sendBroadcast(content: String, channel: String? = null): Result<Unit> =
        action { api.sendBroadcast(SendBroadcastBody(content = content, channel = channel)) }

    // ── Tasks ─────────────────────────────────────────────────────────────────

    suspend fun getTasks(): Result<List<Task>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.getTasks().body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "getTasks failed")
                body.data ?: emptyList()
            }
        }

    suspend fun createTask(description: String): Result<Unit> =
        action { api.createTask(CreateTaskBody(description = description)) }

    suspend fun updateTask(id: String, status: String, reason: String? = null): Result<Unit> =
        action { api.updateTask(id, UpdateTaskBody(status = status, reason = reason)) }

    // ── Dialogues ─────────────────────────────────────────────────────────────

    suspend fun getDialogues(): Result<List<Dialogue>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.getDialogues().body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "getDialogues failed")
                body.data ?: emptyList()
            }
        }

    suspend fun startDialogue(responder: String, topic: String): Result<Unit> =
        action { api.startDialogue(StartDialogueBody(responder = responder, topic = topic)) }

    suspend fun respondDialogue(id: String, response: String): Result<Unit> =
        action { api.respondDialogue(id, RespondDialogueBody(response = response)) }

    // ── Notebook ──────────────────────────────────────────────────────────────

    suspend fun getNotes(limit: Int = 20): Result<List<Note>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.getNotes(limit).body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "getNotes failed")
                body.data ?: emptyList()
            }
        }

    suspend fun rememberNote(content: String, tags: String? = null): Result<Unit> =
        action { api.rememberNote(RememberBody(content = content, tags = tags)) }

    suspend fun recallNotes(query: String): Result<List<NoteSearchResult>> =
        withContext(Dispatchers.IO) {
            runCatching {
                val body = api.recallNotes(query).body() ?: error("Empty response")
                if (!body.ok) error(body.error ?: "recallNotes failed")
                body.data ?: emptyList()
            }
        }

    // ── Unpair ────────────────────────────────────────────────────────────────

    suspend fun unpair(): Result<Unit> = action { api.unpair() }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /** Wraps an action call (ok/error envelope, no data payload). */
    private suspend fun action(
        call: suspend () -> retrofit2.Response<ActionResponse>
    ): Result<Unit> = withContext(Dispatchers.IO) {
        runCatching {
            val body = call().body() ?: error("Empty response")
            if (!body.ok) error(body.error ?: "Request failed")
        }
    }

}
