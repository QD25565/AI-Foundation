package com.aifoundation.app.viewmodel

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.aifoundation.app.data.local.DeepNetPreferences
import com.aifoundation.app.data.model.*
import com.aifoundation.app.data.network.SseClient
import com.aifoundation.app.data.network.TeambookClient
import com.aifoundation.app.data.repository.TeambookRepository
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Central ViewModel for AI-Foundation mobile app.
 *
 * All data is typed — no raw String StateFlows.
 * SSE events update the flows directly without a polling round-trip.
 * Manual refresh methods are available for pull-to-refresh.
 *
 * Pairing state (token, hId, serverUrl) is persisted via DeepNetPreferences
 * so the user stays authenticated across process death / app restarts.
 */
class DeepNetViewModel(application: Application) : AndroidViewModel(application) {

    private val prefs = DeepNetPreferences(application)
    private val repository = TeambookRepository()
    private val sseClient = SseClient()

    // ── Pairing ───────────────────────────────────────────────────────────────

    private val _isPaired = MutableStateFlow(false)
    val isPaired: StateFlow<Boolean> = _isPaired.asStateFlow()

    private val _isPairing = MutableStateFlow(false)
    val isPairing: StateFlow<Boolean> = _isPairing.asStateFlow()

    private val _hId = MutableStateFlow("")
    val hId: StateFlow<String> = _hId.asStateFlow()

    // Initialised from persisted value so the URL field is pre-filled on relaunch
    private val _serverUrl = MutableStateFlow(prefs.teambookServerUrl)
    val serverUrl: StateFlow<String> = _serverUrl.asStateFlow()

    private val _pairingCode = MutableStateFlow<String?>(null)
    val pairingCode: StateFlow<String?> = _pairingCode.asStateFlow()

    private val _pairingError = MutableStateFlow<String?>(null)
    val pairingError: StateFlow<String?> = _pairingError.asStateFlow()

    // ── Team ──────────────────────────────────────────────────────────────────

    private val _team = MutableStateFlow<List<TeamMember>>(emptyList())
    val team: StateFlow<List<TeamMember>> = _team.asStateFlow()

    // ── DMs ───────────────────────────────────────────────────────────────────

    private val _dms = MutableStateFlow<List<Dm>>(emptyList())
    val dms: StateFlow<List<Dm>> = _dms.asStateFlow()

    // ── Broadcasts ────────────────────────────────────────────────────────────

    private val _broadcasts = MutableStateFlow<List<Broadcast>>(emptyList())
    val broadcasts: StateFlow<List<Broadcast>> = _broadcasts.asStateFlow()

    // ── Tasks ─────────────────────────────────────────────────────────────────

    private val _tasks = MutableStateFlow<List<Task>>(emptyList())
    val tasks: StateFlow<List<Task>> = _tasks.asStateFlow()

    // ── Dialogues ─────────────────────────────────────────────────────────────

    private val _dialogues = MutableStateFlow<List<Dialogue>>(emptyList())
    val dialogues: StateFlow<List<Dialogue>> = _dialogues.asStateFlow()

    // ── Notebook ──────────────────────────────────────────────────────────────

    private val _notes = MutableStateFlow<List<Note>>(emptyList())
    val notes: StateFlow<List<Note>> = _notes.asStateFlow()

    private val _noteSearchResults = MutableStateFlow<List<NoteSearchResult>>(emptyList())
    val noteSearchResults: StateFlow<List<NoteSearchResult>> = _noteSearchResults.asStateFlow()

    // ── Pending DM recipient (set when tapping an AI in TeamScreen) ───────────

    private val _pendingDmRecipient = MutableStateFlow<String?>(null)
    val pendingDmRecipient: StateFlow<String?> = _pendingDmRecipient.asStateFlow()

    fun setPendingDmRecipient(aiId: String) { _pendingDmRecipient.value = aiId }
    fun clearPendingDmRecipient() { _pendingDmRecipient.value = null }

    // ── Status ────────────────────────────────────────────────────────────────

    private val _isLoading = MutableStateFlow(false)
    val isLoading: StateFlow<Boolean> = _isLoading.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private val _sseConnected = MutableStateFlow(false)
    val sseConnected: StateFlow<Boolean> = _sseConnected.asStateFlow()

    // ── Restore session on process restart ───────────────────────────────────
    // init runs after ALL properties above are initialized — safe to call startSse/refreshAll.
    init {
        if (prefs.isPaired) {
            val savedUrl   = prefs.teambookServerUrl
            val savedToken = prefs.pairingToken ?: ""
            val savedHId   = prefs.hId ?: ""

            TeambookClient.setServerUrl("$savedUrl/")
            TeambookClient.setAuthToken(savedToken)

            _serverUrl.value = savedUrl
            _hId.value       = savedHId
            _isPaired.value  = true

            startSse()
            refreshAll()
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Pairing
    // ─────────────────────────────────────────────────────────────────────────

    /**
     * Request a pairing code from the server.
     * Display the returned code and instruct the user to run:
     *   teambook mobile-pair <code>
     * Then call [pollPairingCode] periodically until approved.
     */
    fun requestPairingCode(serverUrl: String) {
        viewModelScope.launch {
            _isPairing.value = true
            _pairingError.value = null

            val normalised = serverUrl.trimEnd('/')
            _serverUrl.value = normalised
            prefs.teambookServerUrl = normalised
            TeambookClient.setServerUrl("$normalised/")

            repository.pairRequest()
                .onSuccess { resp -> _pairingCode.value = resp.code }
                .onFailure { e -> _pairingError.value = e.message ?: "Could not reach server" }

            _isPairing.value = false
        }
    }

    /**
     * Poll to check if the pairing code has been approved on the server.
     * Call every ~3s after showing the code. [onSuccess] fires once on approval.
     */
    fun pollPairingCode(code: String, onSuccess: () -> Unit = {}) {
        viewModelScope.launch {
            repository.pairValidate(code)
                .onSuccess { resp ->
                    when {
                        resp.ok && resp.token != null -> {
                            val hId = resp.h_id ?: ""
                            TeambookClient.setAuthToken(resp.token)
                            prefs.savePairing(hId, resp.token)
                            _hId.value = hId
                            _isPaired.value = true
                            _pairingCode.value = null
                            _pairingError.value = null
                            startSse()
                            refreshAll()
                            onSuccess()
                        }
                        resp.pending == true -> { /* not approved yet — keep polling */ }
                        else -> {
                            _pairingError.value = resp.error ?: "Invalid or expired code"
                            _pairingCode.value = null
                        }
                    }
                }
                .onFailure { e -> _pairingError.value = e.message ?: "Network error" }
        }
    }

    fun unpair() {
        viewModelScope.launch {
            stopSse()
            repository.unpair() // best-effort
            prefs.clearPairing()
            TeambookClient.setAuthToken(null)
            _isPaired.value = false
            _hId.value = ""
            _pairingCode.value = null
            _pairingError.value = null
            _team.value = emptyList()
            _dms.value = emptyList()
            _broadcasts.value = emptyList()
            _tasks.value = emptyList()
            _dialogues.value = emptyList()
            _notes.value = emptyList()
            _noteSearchResults.value = emptyList()
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // SSE lifecycle
    // ─────────────────────────────────────────────────────────────────────────

    private fun startSse() {
        val token = TeambookClient.getAuthToken() ?: return
        sseClient.connect(_serverUrl.value, token) { event ->
            viewModelScope.launch { handleSseEvent(event) }
        }
        _sseConnected.value = true
    }

    private fun stopSse() {
        sseClient.disconnect()
        _sseConnected.value = false
    }

    private fun handleSseEvent(event: LiveEvent) {
        when (event) {
            is LiveEvent.DmReceived -> {
                if (_dms.value.none { it.id == event.dm.id }) {
                    _dms.value = listOf(event.dm) + _dms.value
                }
            }
            is LiveEvent.BroadcastReceived -> {
                if (_broadcasts.value.none { it.id == event.bc.id }) {
                    _broadcasts.value = listOf(event.bc) + _broadcasts.value
                }
            }
            is LiveEvent.TeamUpdated -> _team.value = event.members
            is LiveEvent.TaskUpdated -> {
                _tasks.value = _tasks.value.map {
                    if (it.id == event.task.id) event.task else it
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Refresh
    // ─────────────────────────────────────────────────────────────────────────

    fun refreshAll() {
        viewModelScope.launch {
            _isLoading.value = true
            _error.value = null
            try {
                val t  = async { repository.getTeam() }
                val d  = async { repository.getDms() }
                val bc = async { repository.getBroadcasts() }
                val tk = async { repository.getTasks() }
                val dl = async { repository.getDialogues() }
                val n  = async { repository.getNotes() }

                t.await().onSuccess  { _team.value = it }
                d.await().onSuccess  { _dms.value = it }
                bc.await().onSuccess { _broadcasts.value = it }
                tk.await().onSuccess { _tasks.value = it }
                dl.await().onSuccess { _dialogues.value = it }
                n.await().onSuccess  { _notes.value = it }

                // Surface first error encountered (non-fatal — data already updated)
                listOf(t, d, bc, tk, dl, n)
                    .firstOrNull { it.await().isFailure }
                    ?.await()?.exceptionOrNull()
                    ?.let { _error.value = it.message }
            } finally {
                _isLoading.value = false
            }
        }
    }

    fun refreshTeam()       { viewModelScope.launch { repository.getTeam().onSuccess       { _team.value = it } } }
    fun refreshDms()        { viewModelScope.launch { repository.getDms().onSuccess        { _dms.value = it } } }
    fun refreshBroadcasts() { viewModelScope.launch { repository.getBroadcasts().onSuccess { _broadcasts.value = it } } }
    fun refreshTasks()      { viewModelScope.launch { repository.getTasks().onSuccess      { _tasks.value = it } } }
    fun refreshDialogues()  { viewModelScope.launch { repository.getDialogues().onSuccess  { _dialogues.value = it } } }
    fun refreshNotes()      { viewModelScope.launch { repository.getNotes().onSuccess      { _notes.value = it } } }

    // ─────────────────────────────────────────────────────────────────────────
    // Actions
    // ─────────────────────────────────────────────────────────────────────────

    fun sendDm(to: String, content: String, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.sendDm(to, content).also(onResult).onSuccess { refreshDms() }
        }
    }

    fun sendBroadcast(content: String, channel: String? = null, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.sendBroadcast(content, channel).also(onResult).onSuccess { refreshBroadcasts() }
        }
    }

    fun createTask(description: String, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.createTask(description).also(onResult).onSuccess { refreshTasks() }
        }
    }

    fun updateTask(id: String, status: String, reason: String? = null, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.updateTask(id, status, reason).also(onResult).onSuccess { refreshTasks() }
        }
    }

    fun startDialogue(responder: String, topic: String, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.startDialogue(responder, topic).also(onResult).onSuccess { refreshDialogues() }
        }
    }

    fun respondDialogue(id: String, response: String, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.respondDialogue(id, response).also(onResult).onSuccess { refreshDialogues() }
        }
    }

    fun rememberNote(content: String, tags: String? = null, onResult: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch {
            repository.rememberNote(content, tags).also(onResult).onSuccess { refreshNotes() }
        }
    }

    fun recallNotes(query: String) {
        viewModelScope.launch {
            repository.recallNotes(query)
                .onSuccess { _noteSearchResults.value = it }
                .onFailure { _error.value = it.message }
        }
    }

    fun clearError() { _error.value = null }

    // ─────────────────────────────────────────────────────────────────────────
    // Lifecycle
    // ─────────────────────────────────────────────────────────────────────────

    override fun onCleared() {
        super.onCleared()
        stopSse()
    }
}
