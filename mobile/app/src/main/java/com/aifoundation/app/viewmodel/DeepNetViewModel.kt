package com.aifoundation.app.viewmodel

import android.app.Application
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.aifoundation.app.data.local.DeepNetPreferences
import com.aifoundation.app.data.model.*
import com.aifoundation.app.data.network.NetworkClient
import com.aifoundation.app.data.network.TeambookClient
import com.aifoundation.app.data.repository.FederationRepository
import com.aifoundation.app.data.repository.TeambookRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.time.Instant

/**
 * ViewModel for Deep Net state management.
 * Handles both legacy federation operations and new teambook HTTP API.
 */
class DeepNetViewModel(application: Application) : AndroidViewModel(application) {

    companion object {
        private const val TAG = "DeepNetViewModel"
    }

    private val preferences = DeepNetPreferences(application)
    private val repository = FederationRepository()
    private val teambookRepo = TeambookRepository()

    // ============================================================================
    // CONNECTION STATE (legacy federation)
    // ============================================================================

    private val _connectionState = MutableStateFlow(ConnectionState.DISCONNECTED)
    val connectionState: StateFlow<ConnectionState> = _connectionState.asStateFlow()

    private val _wallStatus = MutableStateFlow(WallStatus.SECURE)
    val wallStatus: StateFlow<WallStatus> = _wallStatus.asStateFlow()

    private val _nodes = MutableStateFlow<List<FederationNode>>(emptyList())
    val nodes: StateFlow<List<FederationNode>> = _nodes.asStateFlow()

    private val _stats = MutableStateFlow<FederationStats?>(null)
    val stats: StateFlow<FederationStats?> = _stats.asStateFlow()

    private val _messages = MutableStateFlow<List<DeepNetMessage>>(emptyList())
    val messages: StateFlow<List<DeepNetMessage>> = _messages.asStateFlow()

    private val _deviceId = MutableStateFlow("")
    val deviceId: StateFlow<String> = _deviceId.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private val _serverUrl = MutableStateFlow(preferences.serverUrl)
    val serverUrl: StateFlow<String> = _serverUrl.asStateFlow()

    // ============================================================================
    // TEAMBOOK / HUMAN INTEGRATION STATE
    // ============================================================================

    private val _isPaired = MutableStateFlow(preferences.isPaired)
    val isPaired: StateFlow<Boolean> = _isPaired.asStateFlow()

    private val _isPairing = MutableStateFlow(false)
    val isPairing: StateFlow<Boolean> = _isPairing.asStateFlow()

    private val _hId = MutableStateFlow(preferences.hId ?: "")
    val hId: StateFlow<String> = _hId.asStateFlow()

    private val _teambookServerUrl = MutableStateFlow(preferences.teambookServerUrl)
    val teambookServerUrl: StateFlow<String> = _teambookServerUrl.asStateFlow()

    // Teambook data (CLI text output)
    private val _teamStatus = MutableStateFlow("")
    val teamStatus: StateFlow<String> = _teamStatus.asStateFlow()

    private val _dmsData = MutableStateFlow("")
    val dmsData: StateFlow<String> = _dmsData.asStateFlow()

    private val _broadcastsData = MutableStateFlow("")
    val broadcastsData: StateFlow<String> = _broadcastsData.asStateFlow()

    private val _tasksData = MutableStateFlow("")
    val tasksData: StateFlow<String> = _tasksData.asStateFlow()

    private val _notesData = MutableStateFlow("")
    val notesData: StateFlow<String> = _notesData.asStateFlow()

    private val _searchResults = MutableStateFlow("")
    val searchResults: StateFlow<String> = _searchResults.asStateFlow()

    private val _dialoguesData = MutableStateFlow("")
    val dialoguesData: StateFlow<String> = _dialoguesData.asStateFlow()

    private val _isTeambookLoading = MutableStateFlow(false)
    val isTeambookLoading: StateFlow<Boolean> = _isTeambookLoading.asStateFlow()

    init {
        // Load persisted identity
        preferences.deviceId?.let { savedId ->
            _deviceId.value = savedId
            Log.i(TAG, "Loaded persisted device ID: $savedId")
        }

        // Restore pairing state
        if (preferences.isPaired) {
            val token = preferences.pairingToken
            val url = preferences.teambookServerUrl
            _hId.value = preferences.hId ?: ""
            TeambookClient.setServerUrl(url)
            TeambookClient.setAuthToken(token)
            Log.i(TAG, "Restored pairing: hId=${_hId.value}, url=$url")

            // Auto-refresh data
            refreshTeambook()
        }

        _connectionState.value = ConnectionState.DISCONNECTED
    }

    // ============================================================================
    // PAIRING
    // ============================================================================

    fun pair(serverUrl: String, code: String) {
        viewModelScope.launch {
            _isPairing.value = true
            _error.value = null

            try {
                TeambookClient.setServerUrl(serverUrl)
                preferences.teambookServerUrl = serverUrl
                _teambookServerUrl.value = serverUrl

                val result = teambookRepo.pairValidate(code)

                if (result.isSuccess) {
                    val response = result.getOrNull()!!
                    val hId = response.h_id ?: ""
                    val token = response.token ?: ""

                    preferences.savePairing(hId, token)
                    TeambookClient.setAuthToken(token)
                    _hId.value = hId
                    _isPaired.value = true

                    Log.i(TAG, "Paired as $hId")

                    // Load initial data
                    refreshTeambook()
                } else {
                    _error.value = result.exceptionOrNull()?.message ?: "Pairing failed"
                }
            } catch (e: Exception) {
                Log.e(TAG, "Pairing failed", e)
                _error.value = "Pairing failed: ${e.message}"
            } finally {
                _isPairing.value = false
            }
        }
    }

    fun unpair() {
        preferences.clearPairing()
        TeambookClient.setAuthToken(null)
        _isPaired.value = false
        _hId.value = ""
        _teamStatus.value = ""
        _dmsData.value = ""
        _broadcastsData.value = ""
        _tasksData.value = ""
        _notesData.value = ""
        _dialoguesData.value = ""
        Log.i(TAG, "Unpaired")
    }

    // ============================================================================
    // TEAMBOOK REFRESH
    // ============================================================================

    fun refreshTeambook() {
        viewModelScope.launch {
            _isTeambookLoading.value = true
            try {
                // Fetch all data in parallel
                launch { refreshStatus() }
                launch { refreshDms() }
                launch { refreshBroadcasts() }
                launch { refreshTasks() }
                launch { refreshNotes() }
                launch { refreshDialogues() }
            } finally {
                _isTeambookLoading.value = false
            }
        }
    }

    private suspend fun refreshStatus() {
        teambookRepo.getStatus().onSuccess { _teamStatus.value = it }
    }

    fun refreshDms() {
        viewModelScope.launch {
            teambookRepo.getDms(20).onSuccess { _dmsData.value = it }
        }
    }

    fun refreshBroadcasts() {
        viewModelScope.launch {
            teambookRepo.getBroadcasts(20).onSuccess { _broadcastsData.value = it }
        }
    }

    fun refreshTasks() {
        viewModelScope.launch {
            teambookRepo.getTasks().onSuccess { _tasksData.value = it }
        }
    }

    fun refreshNotes() {
        viewModelScope.launch {
            teambookRepo.notebookList(20).onSuccess { _notesData.value = it }
        }
    }

    fun refreshDialogues() {
        viewModelScope.launch {
            teambookRepo.getDialogues(10).onSuccess { _dialoguesData.value = it }
        }
    }

    // ============================================================================
    // TEAMBOOK ACTIONS
    // ============================================================================

    fun sendTeambookDm(to: String, content: String) {
        viewModelScope.launch {
            teambookRepo.sendDm(to, content).onSuccess {
                refreshDms()
            }.onFailure {
                _error.value = "Send DM failed: ${it.message}"
            }
        }
    }

    fun sendTeambookBroadcast(content: String) {
        viewModelScope.launch {
            teambookRepo.sendBroadcast(content).onSuccess {
                refreshBroadcasts()
            }.onFailure {
                _error.value = "Broadcast failed: ${it.message}"
            }
        }
    }

    fun createTask(description: String) {
        viewModelScope.launch {
            teambookRepo.createTask(description).onSuccess {
                refreshTasks()
            }.onFailure {
                _error.value = "Create task failed: ${it.message}"
            }
        }
    }

    fun updateTask(id: String, status: String) {
        viewModelScope.launch {
            teambookRepo.updateTask(id, status).onSuccess {
                refreshTasks()
            }.onFailure {
                _error.value = "Update task failed: ${it.message}"
            }
        }
    }

    fun notebookRemember(content: String, tags: String?) {
        viewModelScope.launch {
            teambookRepo.notebookRemember(content, tags).onSuccess {
                refreshNotes()
            }.onFailure {
                _error.value = "Save note failed: ${it.message}"
            }
        }
    }

    fun notebookRecall(query: String) {
        viewModelScope.launch {
            teambookRepo.notebookRecall(query).onSuccess {
                _searchResults.value = it
            }.onFailure {
                _error.value = "Search failed: ${it.message}"
            }
        }
    }

    fun startDialogue(responder: String, topic: String) {
        viewModelScope.launch {
            teambookRepo.startDialogue(responder, topic).onSuccess {
                refreshDialogues()
            }.onFailure {
                _error.value = "Start dialogue failed: ${it.message}"
            }
        }
    }

    fun respondDialogue(id: String, response: String) {
        viewModelScope.launch {
            teambookRepo.respondDialogue(id, response).onSuccess {
                refreshDialogues()
            }.onFailure {
                _error.value = "Respond failed: ${it.message}"
            }
        }
    }

    // ============================================================================
    // LEGACY FEDERATION METHODS
    // ============================================================================

    fun connect(serverUrl: String) {
        viewModelScope.launch {
            _connectionState.value = ConnectionState.CONNECTING
            _wallStatus.value = WallStatus.VERIFYING
            _serverUrl.value = serverUrl
            preferences.serverUrl = serverUrl

            try {
                NetworkClient.setServerUrl(serverUrl)

                val healthResult = repository.healthCheck()
                if (healthResult.isFailure) {
                    _error.value = "Server unreachable: ${healthResult.exceptionOrNull()?.message}"
                    _connectionState.value = ConnectionState.DISCONNECTED
                    _wallStatus.value = WallStatus.SECURE
                    return@launch
                }

                _connectionState.value = ConnectionState.CONNECTED

                val existingDeviceId = preferences.deviceId
                val existingFingerprint = preferences.fingerprint
                val deviceName = preferences.deviceName ?: "Android-${android.os.Build.MODEL}"

                if (existingDeviceId != null && existingFingerprint != null) {
                    val reconnectResult = repository.reconnect(existingDeviceId, existingFingerprint)
                    if (reconnectResult.isSuccess) {
                        _deviceId.value = existingDeviceId
                        _connectionState.value = ConnectionState.AUTHENTICATED
                        _wallStatus.value = WallStatus.SECURE
                        preferences.updateLastConnected()
                        refreshFederation()
                        refreshMessages()
                        return@launch
                    } else {
                        preferences.clearIdentity()
                    }
                }

                val registerResult = repository.register(deviceName, "mobile")
                if (registerResult.isSuccess) {
                    val registration = registerResult.getOrNull()!!
                    _deviceId.value = registration.deviceId
                    _connectionState.value = ConnectionState.AUTHENTICATED
                    _wallStatus.value = WallStatus.SECURE
                    preferences.saveRegistration(registration.deviceId, deviceName, registration.fingerprint)
                    refreshFederation()
                    refreshMessages()
                } else {
                    _error.value = "Registration failed: ${registerResult.exceptionOrNull()?.message}"
                    _connectionState.value = ConnectionState.CONNECTED
                    _wallStatus.value = WallStatus.SECURE
                }
            } catch (e: Exception) {
                Log.e(TAG, "Connection failed", e)
                _error.value = "Connection failed: ${e.message}"
                _connectionState.value = ConnectionState.DISCONNECTED
                _wallStatus.value = WallStatus.SECURE
            }
        }
    }

    fun disconnect() {
        viewModelScope.launch {
            _connectionState.value = ConnectionState.DISCONNECTED
            _nodes.value = emptyList()
            _stats.value = null
            _messages.value = emptyList()
        }
    }

    fun logout() {
        viewModelScope.launch {
            preferences.clearIdentity()
            _connectionState.value = ConnectionState.DISCONNECTED
            _nodes.value = emptyList()
            _stats.value = null
            _messages.value = emptyList()
            _deviceId.value = ""
        }
    }

    fun refreshFederation() {
        viewModelScope.launch {
            try {
                val membersResult = repository.getMembers()
                if (membersResult.isSuccess) {
                    _nodes.value = membersResult.getOrNull() ?: emptyList()
                }

                val teamResult = repository.getTeam()
                if (teamResult.isSuccess) {
                    val team = teamResult.getOrNull() ?: emptyList()
                    val updatedNodes = _nodes.value.map { node ->
                        team.find { it.id == node.id }?.let { teamMember ->
                            node.copy(currentActivity = teamMember.currentActivity)
                        } ?: node
                    }
                    _nodes.value = updatedNodes
                }

                val statusResult = repository.getStatus()
                if (statusResult.isSuccess) {
                    val status = statusResult.getOrNull()!!
                    val nodeList = _nodes.value
                    _stats.value = FederationStats(
                        totalNodes = nodeList.size,
                        aiAgents = nodeList.count { it.entityType == EntityType.AI_AGENT },
                        humanUsers = nodeList.count {
                            it.entityType == EntityType.HUMAN_MOBILE ||
                            it.entityType == EntityType.HUMAN_DESKTOP
                        },
                        servers = nodeList.count { it.entityType == EntityType.SERVER },
                        messagesLast24h = 0,
                        uptime = status.serverUptimeSecs
                    )
                }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to refresh federation", e)
                _error.value = "Refresh failed: ${e.message}"
            }
        }
    }

    fun sendMessage(content: String, recipientId: String?) {
        viewModelScope.launch {
            try {
                val result = if (recipientId.isNullOrBlank()) {
                    repository.sendBroadcast(content)
                } else {
                    repository.sendDm(recipientId, content)
                }

                if (result.isSuccess) {
                    val messageId = result.getOrNull() ?: 0L
                    val newMessage = DeepNetMessage(
                        id = messageId,
                        from = _deviceId.value,
                        to = recipientId,
                        content = content,
                        timestamp = Instant.now(),
                        messageType = if (recipientId == null) MessageType.BROADCAST else MessageType.DIRECT
                    )
                    _messages.value = listOf(newMessage) + _messages.value
                } else {
                    _error.value = "Send failed: ${result.exceptionOrNull()?.message}"
                }
            } catch (e: Exception) {
                _error.value = "Send failed: ${e.message}"
            }
        }
    }

    fun refreshMessages() {
        viewModelScope.launch {
            try {
                val result = repository.getMessages(50)
                if (result.isSuccess) {
                    _messages.value = result.getOrNull() ?: emptyList()
                }
            } catch (e: Exception) {
                _error.value = "Refresh messages failed: ${e.message}"
            }
        }
    }

    fun clearError() {
        _error.value = null
    }

    fun setServerUrl(url: String) {
        _serverUrl.value = url
    }

    fun setTeambookServerUrl(url: String) {
        _teambookServerUrl.value = url
    }
}
