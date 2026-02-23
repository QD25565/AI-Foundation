package com.aifoundation.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.MenuBook
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
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.screens.*
import com.aifoundation.app.ui.theme.AIFoundationTheme
import com.aifoundation.app.ui.theme.DeepNetColors
import com.aifoundation.app.viewmodel.DeepNetViewModel
import kotlinx.coroutines.delay

/**
 * AI-Foundation Mobile — Human Interface to the AI team.
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
    val isPaired     by viewModel.isPaired.collectAsState()
    val isPairing    by viewModel.isPairing.collectAsState()
    val pairingCode  by viewModel.pairingCode.collectAsState()
    val pairingError by viewModel.pairingError.collectAsState()
    val serverUrl    by viewModel.serverUrl.collectAsState()

    // Auto-poll every 3 s while a pairing code is outstanding.
    // The coroutine is cancelled and restarted whenever pairingCode changes.
    LaunchedEffect(pairingCode) {
        pairingCode?.let { code ->
            while (true) {
                delay(3_000)
                viewModel.pollPairingCode(code)
            }
        }
    }

    if (!isPaired) {
        PairingScreen(
            serverUrl     = serverUrl,
            onRequestCode = { url -> viewModel.requestPairingCode(url) },
            pairingCode   = pairingCode,
            isPairing     = isPairing,
            pairingError  = pairingError,
            onClearError  = { viewModel.clearError() }
        )
    } else {
        DeepNetApp(viewModel = viewModel)
    }
}

// ── Navigation destinations ────────────────────────────────────────────────────

sealed class Screen(val route: String, val title: String, val icon: @Composable () -> Unit) {
    object Inbox    : Screen("inbox",    "Inbox",    { Icon(Icons.Default.Inbox,    contentDescription = null) })
    object Team     : Screen("team",     "Team",     { Icon(Icons.Default.Groups,   contentDescription = null) })
    object Tasks    : Screen("tasks",    "Tasks",    { Icon(Icons.Default.TaskAlt,  contentDescription = null) })
    object Notes    : Screen("notes",    "Notes",    { Icon(Icons.AutoMirrored.Filled.MenuBook, contentDescription = null) })
    object Settings : Screen("settings", "Settings", { Icon(Icons.Default.Settings, contentDescription = null) })
}

// ── Main app shell ─────────────────────────────────────────────────────────────

@Composable
fun DeepNetApp(viewModel: DeepNetViewModel) {
    val navController     = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute      = navBackStackEntry?.destination?.route

    val screens = listOf(Screen.Inbox, Screen.Team, Screen.Tasks, Screen.Notes, Screen.Settings)

    // Hide bottom nav when inside a conversation (full-screen chat).
    val showBottomNav = currentRoute?.startsWith("conversation/") != true

    // Collect all typed state in one place
    val hId           by viewModel.hId.collectAsState()
    val serverUrl     by viewModel.serverUrl.collectAsState()
    val team          by viewModel.team.collectAsState()
    val dms           by viewModel.dms.collectAsState()
    val broadcasts    by viewModel.broadcasts.collectAsState()
    val tasks         by viewModel.tasks.collectAsState()
    val dialogues     by viewModel.dialogues.collectAsState()
    val notes         by viewModel.notes.collectAsState()
    val noteSearchResults by viewModel.noteSearchResults.collectAsState()
    val isLoading     by viewModel.isLoading.collectAsState()
    val sseConnected  by viewModel.sseConnected.collectAsState()
    val error         by viewModel.error.collectAsState()

    val snackbarHostState = remember { SnackbarHostState() }
    LaunchedEffect(error) {
        error?.let {
            snackbarHostState.showSnackbar(it)
            viewModel.clearError()
        }
    }

    Scaffold(
        containerColor = DeepNetColors.Background,
        snackbarHost   = { SnackbarHost(snackbarHostState) },
        bottomBar = {
            if (showBottomNav) {
                NavigationBar(
                    containerColor = DeepNetColors.Surface,
                    contentColor   = DeepNetColors.OnSurface
                ) {
                    screens.forEach { screen ->
                        NavigationBarItem(
                            icon     = screen.icon,
                            label    = { Text(screen.title, fontSize = 10.sp, fontFamily = FontFamily.Monospace) },
                            selected = currentRoute == screen.route,
                            onClick  = {
                                if (currentRoute != screen.route) {
                                    navController.navigate(screen.route) {
                                        popUpTo(navController.graph.startDestinationId)
                                        launchSingleTop = true
                                    }
                                }
                            },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor   = DeepNetColors.Primary,
                                selectedTextColor   = DeepNetColors.Primary,
                                unselectedIconColor = DeepNetColors.OnSurfaceVariant,
                                unselectedTextColor = DeepNetColors.OnSurfaceVariant,
                                indicatorColor      = DeepNetColors.Primary.copy(alpha = 0.1f)
                            )
                        )
                    }
                }
            }
        }
    ) { paddingValues ->
        NavHost(
            navController    = navController,
            startDestination = Screen.Inbox.route,
            modifier         = Modifier.padding(paddingValues)
        ) {
            // Inbox: DMs (threaded) + Broadcasts + Dialogues
            composable(Screen.Inbox.route) {
                InboxScreen(
                    dms                = dms,
                    broadcasts         = broadcasts,
                    dialogues          = dialogues,
                    myHId              = hId,
                    team               = team,
                    onOpenConversation = { partnerId ->
                        navController.navigate("conversation/$partnerId") {
                            launchSingleTop = true
                        }
                    },
                    onRefreshDms        = { viewModel.refreshDms() },
                    onRefreshBroadcasts = { viewModel.refreshBroadcasts() },
                    onRefreshDialogues  = { viewModel.refreshDialogues() },
                    onSendBroadcast     = { content -> viewModel.sendBroadcast(content) },
                    onStartDialogue     = { responder, topic -> viewModel.startDialogue(responder, topic) },
                    onRespondDialogue   = { id, response -> viewModel.respondDialogue(id, response) },
                    isLoading           = isLoading
                )
            }

            // Team: AI + Human roster with live presence. Tapping an AI opens conversation.
            composable(Screen.Team.route) {
                TeamScreen(
                    team      = team,
                    onRefresh = { viewModel.refreshTeam() },
                    onSendDm  = { aiId ->
                        navController.navigate("conversation/$aiId") {
                            launchSingleTop = true
                        }
                    },
                    isLoading = isLoading
                )
            }

            composable(Screen.Tasks.route) {
                TasksScreen(
                    tasks        = tasks,
                    onRefresh    = { viewModel.refreshTasks() },
                    onCreateTask = { desc -> viewModel.createTask(desc) },
                    onUpdateTask = { id, status -> viewModel.updateTask(id, status) },
                    isLoading    = isLoading
                )
            }

            composable(Screen.Notes.route) {
                NotebookScreen(
                    notes             = notes,
                    noteSearchResults = noteSearchResults,
                    onRefresh         = { viewModel.refreshNotes() },
                    onRemember        = { content, tags -> viewModel.rememberNote(content, tags) },
                    onRecall          = { query -> viewModel.recallNotes(query) },
                    isLoading         = isLoading
                )
            }

            composable(Screen.Settings.route) {
                SettingsScreen(
                    hId          = hId,
                    serverUrl    = serverUrl,
                    sseConnected = sseConnected,
                    onUnpair     = { viewModel.unpair() }
                )
            }

            // Full-screen conversation — bottom nav hidden while here.
            composable("conversation/{partnerId}") { backStackEntry ->
                val partnerId = backStackEntry.arguments?.getString("partnerId") ?: return@composable
                ConversationScreen(
                    partnerId = partnerId,
                    myHId     = hId,
                    allDms    = dms,
                    team      = team,
                    onSendDm  = { to, content -> viewModel.sendDm(to, content) },
                    onBack    = { navController.popBackStack() }
                )
            }
        }
    }
}

// ── Settings screen ────────────────────────────────────────────────────────────

@Composable
fun SettingsScreen(
    hId: String,
    serverUrl: String,
    sseConnected: Boolean,
    onUnpair: () -> Unit
) {
    var showUnpairConfirm by remember { mutableStateOf(false) }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        item {
            Text(
                text  = "SETTINGS",
                style = MaterialTheme.typography.headlineMedium,
                fontFamily = FontFamily.Monospace,
                color = DeepNetColors.Primary
            )
        }

        // Identity + connection
        item {
            DeepNetCard(
                modifier   = Modifier.fillMaxWidth(),
                variant    = DeepNetCardVariant.NODE,
                enableGlow = true
            ) {
                Text(
                    text  = "IDENTITY",
                    style = MaterialTheme.typography.labelMedium,
                    color = DeepNetColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text       = hId.ifBlank { "—" },
                    style      = MaterialTheme.typography.bodyLarge,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color      = DeepNetColors.Primary
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text       = serverUrl,
                    style      = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color      = DeepNetColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(10.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    DeepNetStatusIndicator(
                        status    = if (sseConnected) DeepNetStatus.ONLINE else DeepNetStatus.OFFLINE,
                        showLabel = true,
                        animated  = sseConnected
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        text       = if (sseConnected) "Live updates active" else "Disconnected",
                        style      = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                }
            }
        }

        // Unpair
        item {
            DeepNetButton(
                onClick  = { showUnpairConfirm = true },
                variant  = DeepNetButtonVariant.DANGER,
                icon     = Icons.Default.LinkOff,
                text     = "UNPAIR DEVICE",
                modifier = Modifier.fillMaxWidth()
            )
        }

        // About
        item {
            DeepNetCard(modifier = Modifier.fillMaxWidth(), variant = DeepNetCardVariant.DATA) {
                Text(
                    text  = "AI-Foundation Mobile v2.0",
                    style = MaterialTheme.typography.titleMedium,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.OnSurface
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text  = "Human Interface · Real-time SSE · Threaded DMs",
                    style = MaterialTheme.typography.bodySmall,
                    color = DeepNetColors.OnSurfaceVariant
                )
            }
        }
    }

    // Unpair confirmation dialog
    if (showUnpairConfirm) {
        AlertDialog(
            onDismissRequest = { showUnpairConfirm = false },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(
                    text       = "UNPAIR DEVICE",
                    fontFamily = FontFamily.Monospace,
                    color      = DeepNetColors.Error
                )
            },
            text = {
                Text(
                    text  = "This will disconnect this device from $hId. You will need a new pairing code to reconnect.",
                    color = DeepNetColors.OnSurface
                )
            },
            confirmButton = {
                DeepNetButton(
                    onClick = { onUnpair(); showUnpairConfirm = false },
                    variant = DeepNetButtonVariant.DANGER,
                    text    = "UNPAIR"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showUnpairConfirm = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text    = "CANCEL"
                )
            }
        )
    }
}
