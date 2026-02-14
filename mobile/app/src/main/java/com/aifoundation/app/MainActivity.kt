package com.aifoundation.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.aifoundation.app.data.model.ConnectionState
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.screens.*
import com.aifoundation.app.ui.theme.AIFoundationTheme
import com.aifoundation.app.ui.theme.DeepNetColors
import com.aifoundation.app.viewmodel.DeepNetViewModel

/**
 * Deep Net Mobile Client
 * Android interface to the AI-Foundation teambook
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            AIFoundationTheme {
                DeepNetRoot()
            }
        }
    }
}

@Composable
fun DeepNetRoot(viewModel: DeepNetViewModel = viewModel()) {
    val isPaired by viewModel.isPaired.collectAsState()
    val isPairing by viewModel.isPairing.collectAsState()
    val error by viewModel.error.collectAsState()
    val teambookServerUrl by viewModel.teambookServerUrl.collectAsState()

    if (!isPaired) {
        PairingScreen(
            serverUrl = teambookServerUrl,
            onServerUrlChange = { viewModel.setTeambookServerUrl(it) },
            onPair = { url, code -> viewModel.pair(url, code) },
            isPairing = isPairing,
            error = error,
            onClearError = { viewModel.clearError() }
        )
    } else {
        DeepNetApp(viewModel = viewModel)
    }
}

sealed class Screen(val route: String, val title: String, val icon: @Composable () -> Unit) {
    object Inbox : Screen(
        "inbox",
        "Inbox",
        { Icon(Icons.Default.Inbox, contentDescription = null) }
    )
    object Tasks : Screen(
        "tasks",
        "Tasks",
        { Icon(Icons.Default.TaskAlt, contentDescription = null) }
    )
    object Notes : Screen(
        "notes",
        "Notes",
        { Icon(Icons.Default.MenuBook, contentDescription = null) }
    )
    object Dialogues : Screen(
        "dialogues",
        "Dialogues",
        { Icon(Icons.Default.Forum, contentDescription = null) }
    )
    object Settings : Screen(
        "settings",
        "Settings",
        { Icon(Icons.Default.Settings, contentDescription = null) }
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DeepNetApp(viewModel: DeepNetViewModel) {
    val navController = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = navBackStackEntry?.destination?.route

    val screens = listOf(Screen.Inbox, Screen.Tasks, Screen.Notes, Screen.Dialogues, Screen.Settings)

    // Collect teambook state
    val hId by viewModel.hId.collectAsState()
    val teamStatus by viewModel.teamStatus.collectAsState()
    val dmsData by viewModel.dmsData.collectAsState()
    val broadcastsData by viewModel.broadcastsData.collectAsState()
    val tasksData by viewModel.tasksData.collectAsState()
    val notesData by viewModel.notesData.collectAsState()
    val searchResults by viewModel.searchResults.collectAsState()
    val dialoguesData by viewModel.dialoguesData.collectAsState()
    val isTeambookLoading by viewModel.isTeambookLoading.collectAsState()
    val error by viewModel.error.collectAsState()
    val teambookServerUrl by viewModel.teambookServerUrl.collectAsState()

    // Legacy state (for settings)
    val connectionState by viewModel.connectionState.collectAsState()
    val deviceId by viewModel.deviceId.collectAsState()
    val serverUrl by viewModel.serverUrl.collectAsState()

    // Error snackbar
    val snackbarHostState = remember(Unit) { SnackbarHostState() }
    LaunchedEffect(error) {
        error?.let {
            snackbarHostState.showSnackbar(it)
            viewModel.clearError()
        }
    }

    Scaffold(
        containerColor = DeepNetColors.Background,
        snackbarHost = { SnackbarHost(snackbarHostState) },
        bottomBar = {
            NavigationBar(
                containerColor = DeepNetColors.Surface,
                contentColor = DeepNetColors.OnSurface
            ) {
                screens.forEach { screen ->
                    NavigationBarItem(
                        icon = screen.icon,
                        label = {
                            Text(
                                screen.title,
                                fontSize = 10.sp,
                                fontFamily = FontFamily.Monospace
                            )
                        },
                        selected = currentRoute == screen.route,
                        onClick = {
                            if (currentRoute != screen.route) {
                                navController.navigate(screen.route) {
                                    popUpTo(navController.graph.startDestinationId)
                                    launchSingleTop = true
                                }
                            }
                        },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = DeepNetColors.Primary,
                            selectedTextColor = DeepNetColors.Primary,
                            unselectedIconColor = DeepNetColors.OnSurfaceVariant,
                            unselectedTextColor = DeepNetColors.OnSurfaceVariant,
                            indicatorColor = DeepNetColors.Primary.copy(alpha = 0.1f)
                        )
                    )
                }
            }
        }
    ) { paddingValues ->
        NavHost(
            navController = navController,
            startDestination = Screen.Inbox.route,
            modifier = Modifier.padding(paddingValues)
        ) {
            composable(Screen.Inbox.route) {
                InboxScreen(
                    dmsData = dmsData,
                    broadcastsData = broadcastsData,
                    onRefreshDms = { viewModel.refreshDms() },
                    onRefreshBroadcasts = { viewModel.refreshBroadcasts() },
                    onSendDm = { to, content -> viewModel.sendTeambookDm(to, content) },
                    onSendBroadcast = { content -> viewModel.sendTeambookBroadcast(content) },
                    isLoading = isTeambookLoading
                )
            }

            composable(Screen.Tasks.route) {
                TasksScreen(
                    tasksData = tasksData,
                    onRefresh = { viewModel.refreshTasks() },
                    onCreateTask = { desc -> viewModel.createTask(desc) },
                    onUpdateTask = { id, status -> viewModel.updateTask(id, status) },
                    isLoading = isTeambookLoading
                )
            }

            composable(Screen.Notes.route) {
                NotebookScreen(
                    notesData = notesData,
                    searchResults = searchResults,
                    onRefresh = { viewModel.refreshNotes() },
                    onRemember = { content, tags -> viewModel.notebookRemember(content, tags) },
                    onRecall = { query -> viewModel.notebookRecall(query) },
                    isLoading = isTeambookLoading
                )
            }

            composable(Screen.Dialogues.route) {
                DialoguesScreen(
                    dialoguesData = dialoguesData,
                    onRefresh = { viewModel.refreshDialogues() },
                    onStartDialogue = { responder, topic ->
                        viewModel.startDialogue(responder, topic)
                    },
                    onRespondDialogue = { id, response ->
                        viewModel.respondDialogue(id, response)
                    },
                    isLoading = isTeambookLoading
                )
            }

            composable(Screen.Settings.route) {
                SettingsScreen(
                    hId = hId,
                    teambookServerUrl = teambookServerUrl,
                    teamStatus = teamStatus,
                    onRefreshStatus = { viewModel.refreshTeambook() },
                    onUnpair = { viewModel.unpair() },
                    connectionState = connectionState,
                    deviceId = deviceId,
                    serverUrl = serverUrl,
                    onServerUrlChange = { viewModel.setServerUrl(it) },
                    onConnect = { url -> viewModel.connect(url) },
                    onDisconnect = { viewModel.disconnect() }
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    hId: String,
    teambookServerUrl: String,
    teamStatus: String,
    onRefreshStatus: () -> Unit,
    onUnpair: () -> Unit,
    connectionState: ConnectionState,
    deviceId: String,
    serverUrl: String,
    onServerUrlChange: (String) -> Unit,
    onConnect: (String) -> Unit,
    onDisconnect: () -> Unit
) {
    var editableUrl by remember(serverUrl) { mutableStateOf(serverUrl) }
    var showUnpairConfirm by remember { mutableStateOf(false) }

    LazyColumnSettings(
        hId = hId,
        teambookServerUrl = teambookServerUrl,
        teamStatus = teamStatus,
        onRefreshStatus = onRefreshStatus,
        showUnpairConfirm = showUnpairConfirm,
        onShowUnpairConfirm = { showUnpairConfirm = it },
        onUnpair = onUnpair,
        connectionState = connectionState,
        deviceId = deviceId,
        editableUrl = editableUrl,
        onEditableUrlChange = { editableUrl = it },
        onServerUrlChange = onServerUrlChange,
        onConnect = onConnect,
        onDisconnect = onDisconnect
    )
}

@Composable
private fun LazyColumnSettings(
    hId: String,
    teambookServerUrl: String,
    teamStatus: String,
    onRefreshStatus: () -> Unit,
    showUnpairConfirm: Boolean,
    onShowUnpairConfirm: (Boolean) -> Unit,
    onUnpair: () -> Unit,
    connectionState: ConnectionState,
    deviceId: String,
    editableUrl: String,
    onEditableUrlChange: (String) -> Unit,
    onServerUrlChange: (String) -> Unit,
    onConnect: (String) -> Unit,
    onDisconnect: () -> Unit
) {
    androidx.compose.foundation.lazy.LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        item {
            Text(
                text = "SETTINGS",
                style = MaterialTheme.typography.headlineMedium,
                fontFamily = FontFamily.Monospace,
                color = DeepNetColors.Primary
            )
        }

        // Paired identity
        item {
            DeepNetCard(
                modifier = Modifier.fillMaxWidth(),
                variant = DeepNetCardVariant.NODE,
                enableGlow = true
            ) {
                Text(
                    text = "Human ID",
                    style = MaterialTheme.typography.labelMedium,
                    color = DeepNetColors.OnSurfaceVariant
                )
                Text(
                    text = hId,
                    style = MaterialTheme.typography.bodyLarge,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color = DeepNetColors.Primary
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = "Server: $teambookServerUrl",
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.OnSurfaceVariant
                )
            }
        }

        // Team status
        if (teamStatus.isNotBlank()) {
            item {
                DeepNetCard(
                    modifier = Modifier.fillMaxWidth(),
                    variant = DeepNetCardVariant.DATA
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = "Team Status",
                            style = MaterialTheme.typography.labelMedium,
                            color = DeepNetColors.OnSurfaceVariant
                        )
                        DeepNetButton(
                            onClick = onRefreshStatus,
                            variant = DeepNetButtonVariant.GHOST,
                            icon = Icons.Default.Refresh,
                            text = "REFRESH"
                        )
                    }
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = teamStatus,
                        fontFamily = FontFamily.Monospace,
                        fontSize = 12.sp,
                        color = DeepNetColors.OnSurface,
                        lineHeight = 18.sp
                    )
                }
            }
        }

        // Legacy federation connection
        item {
            DeepNetCard(
                modifier = Modifier.fillMaxWidth(),
                variant = DeepNetCardVariant.TERMINAL
            ) {
                Text(
                    text = "Federation Server (Legacy)",
                    style = MaterialTheme.typography.labelMedium,
                    color = DeepNetColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(8.dp))
                OutlinedTextField(
                    value = editableUrl,
                    onValueChange = onEditableUrlChange,
                    placeholder = { Text("http://192.168.x.x:31415") },
                    modifier = Modifier.fillMaxWidth(),
                    enabled = connectionState == ConnectionState.DISCONNECTED,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = DeepNetColors.Primary,
                        unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                        cursorColor = DeepNetColors.Primary,
                        focusedTextColor = DeepNetColors.OnSurface,
                        unfocusedTextColor = DeepNetColors.OnSurface
                    ),
                    singleLine = true
                )
                Spacer(modifier = Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    DeepNetStatusIndicator(
                        status = when (connectionState) {
                            ConnectionState.AUTHENTICATED -> DeepNetStatus.ONLINE
                            ConnectionState.CONNECTED,
                            ConnectionState.CONNECTING -> DeepNetStatus.CONNECTING
                            ConnectionState.DISCONNECTED -> DeepNetStatus.OFFLINE
                            ConnectionState.ERROR -> DeepNetStatus.ERROR
                        },
                        showLabel = true,
                        animated = connectionState == ConnectionState.CONNECTING
                    )
                    when (connectionState) {
                        ConnectionState.DISCONNECTED -> {
                            DeepNetButton(
                                onClick = {
                                    onServerUrlChange(editableUrl)
                                    onConnect(editableUrl)
                                },
                                variant = DeepNetButtonVariant.PRIMARY,
                                icon = Icons.Default.Link,
                                text = "CONNECT"
                            )
                        }
                        ConnectionState.CONNECTING -> {
                            DeepNetLoadingIndicator()
                        }
                        else -> {
                            DeepNetButton(
                                onClick = onDisconnect,
                                variant = DeepNetButtonVariant.DANGER,
                                icon = Icons.Default.LinkOff,
                                text = "DISCONNECT"
                            )
                        }
                    }
                }
                if (deviceId.isNotEmpty()) {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = "Device: $deviceId",
                        fontFamily = FontFamily.Monospace,
                        fontSize = 11.sp,
                        color = DeepNetColors.OnSurfaceVariant
                    )
                }
            }
        }

        // Unpair button
        item {
            DeepNetButton(
                onClick = { onShowUnpairConfirm(true) },
                variant = DeepNetButtonVariant.DANGER,
                icon = Icons.Default.LinkOff,
                text = "UNPAIR DEVICE",
                modifier = Modifier.fillMaxWidth()
            )
        }

        // About
        item {
            DeepNetCard(
                modifier = Modifier.fillMaxWidth(),
                variant = DeepNetCardVariant.DATA
            ) {
                Text(
                    text = "Deep Net Mobile Client v2.0",
                    style = MaterialTheme.typography.titleMedium,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.OnSurface
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = "AI-Foundation | Human Integration",
                    style = MaterialTheme.typography.bodySmall,
                    color = DeepNetColors.OnSurfaceVariant
                )
            }
        }
    }

    // Unpair confirmation
    if (showUnpairConfirm) {
        AlertDialog(
            onDismissRequest = { onShowUnpairConfirm(false) },
            containerColor = DeepNetColors.Surface,
            title = {
                Text(
                    text = "UNPAIR DEVICE",
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Error
                )
            },
            text = {
                Text(
                    text = "This will disconnect this device from $hId. You'll need a new pairing code to reconnect.",
                    color = DeepNetColors.OnSurface
                )
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        onUnpair()
                        onShowUnpairConfirm(false)
                    },
                    variant = DeepNetButtonVariant.DANGER,
                    text = "UNPAIR"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { onShowUnpairConfirm(false) },
                    variant = DeepNetButtonVariant.GHOST,
                    text = "CANCEL"
                )
            }
        )
    }
}
