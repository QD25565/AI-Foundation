package com.aifoundation.app.data.network

import com.aifoundation.app.data.model.*
import com.google.gson.Gson
import com.google.gson.JsonSyntaxException
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.sse.EventSource
import okhttp3.sse.EventSourceListener
import okhttp3.sse.EventSources
import android.util.Log

/**
 * SSE client for GET /api/events.
 *
 * Establishes a persistent Server-Sent Events connection and converts named
 * events from the server into [LiveEvent] sealed class instances.
 *
 * Usage:
 *   sseClient.connect(baseUrl, token) { event -> /* update StateFlow */ }
 *   sseClient.disconnect()
 *
 * Thread-safety: connect/disconnect may be called from any thread.
 */
class SseClient {

    private val gson = Gson()
    private var eventSource: EventSource? = null

    @Volatile private var connected = false

    fun connect(baseUrl: String, token: String, onEvent: (LiveEvent) -> Unit) {
        disconnect() // clean up any previous connection

        val url = baseUrl.trimEnd('/') + "/api/events"
        val request = Request.Builder()
            .url(url)
            .addHeader("Authorization", "Bearer $token")
            .addHeader("Accept", "text/event-stream")
            .addHeader("Cache-Control", "no-cache")
            .build()

        val client: OkHttpClient = TeambookClient.sseOkHttpClient()

        eventSource = EventSources.createFactory(client)
            .newEventSource(request, object : EventSourceListener() {

                override fun onOpen(eventSource: EventSource, response: Response) {
                    connected = true
                    Log.d(TAG, "SSE connected to $url")
                }

                override fun onEvent(
                    eventSource: EventSource,
                    id: String?,
                    type: String?,
                    data: String
                ) {
                    if (data.isBlank() || type == "keepalive") return
                    val event = parseEvent(type, data) ?: return
                    onEvent(event)
                }

                override fun onClosed(eventSource: EventSource) {
                    connected = false
                    Log.d(TAG, "SSE closed")
                }

                override fun onFailure(
                    eventSource: EventSource,
                    t: Throwable?,
                    response: Response?
                ) {
                    connected = false
                    Log.w(TAG, "SSE failure: ${t?.message ?: response?.code}")
                    // OkHttp won't retry automatically. The ViewModel should
                    // reconnect after pairing is re-confirmed or on next app foreground.
                }
            })
    }

    fun disconnect() {
        eventSource?.cancel()
        eventSource = null
        connected = false
    }

    val isConnected: Boolean get() = connected

    // ── Parsing ───────────────────────────────────────────────────────────────

    private fun parseEvent(type: String?, data: String): LiveEvent? {
        return try {
            when (type) {
                "dm_received" -> {
                    val dm = gson.fromJson(data, Dm::class.java)
                    LiveEvent.DmReceived(dm)
                }
                "broadcast_received" -> {
                    val bc = gson.fromJson(data, Broadcast::class.java)
                    LiveEvent.BroadcastReceived(bc)
                }
                "team_updated" -> {
                    val members = gson.fromJson(data, Array<TeamMember>::class.java).toList()
                    LiveEvent.TeamUpdated(members)
                }
                "task_updated" -> {
                    val task = gson.fromJson(data, Task::class.java)
                    LiveEvent.TaskUpdated(task)
                }
                else -> {
                    Log.v(TAG, "Unknown SSE event type: $type")
                    null
                }
            }
        } catch (e: JsonSyntaxException) {
            Log.w(TAG, "Failed to parse SSE event '$type': ${e.message}")
            null
        }
    }

    companion object {
        private const val TAG = "SseClient"
    }
}
