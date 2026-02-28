package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Reply
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.Dialogue
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors

/**
 * Dialogues screen — standalone view of all structured AI-to-AI conversations.
 * Note: Dialogues are also accessible via the Inbox screen (3rd tab).
 * This screen is kept compiled for potential deep-link / future nav use.
 */
@Composable
fun DialoguesScreen(
    dialogues: List<Dialogue>,
    onRefresh: () -> Unit,
    onStartDialogue: (String, String) -> Unit,
    onRespondDialogue: (String, String) -> Unit,
    isLoading: Boolean
) {
    var showStartDialog   by remember { mutableStateOf(false) }
    var showRespondDialog by remember { mutableStateOf(false) }
    var responder         by remember { mutableStateOf("") }
    var topic             by remember { mutableStateOf("") }
    var selectedId        by remember { mutableStateOf("") }
    var responseText      by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // Header
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text       = "DIALOGUES",
                style      = MaterialTheme.typography.headlineSmall,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Black,
                color      = DeepNetColors.Primary
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                DeepNetButton(onClick = onRefresh, variant = DeepNetButtonVariant.GHOST,
                    icon = Icons.Default.Refresh, text = "REFRESH")
                DeepNetButton(onClick = { showStartDialog = true }, variant = DeepNetButtonVariant.PRIMARY,
                    icon = Icons.Default.Add, text = "START")
            }
        }

        if (isLoading) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetLoadingIndicator(text = "LOADING DIALOGUES...")
            }
            return@Column
        }

        if (dialogues.isEmpty()) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetCard(modifier = Modifier.fillMaxWidth(0.75f), variant = DeepNetCardVariant.TERMINAL) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.fillMaxWidth().padding(32.dp)
                    ) {
                        Icon(imageVector = Icons.Default.Forum, contentDescription = null,
                            tint = DeepNetColors.OnSurfaceVariant, modifier = Modifier.size(48.dp))
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(text = "NO DIALOGUES", fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold, color = DeepNetColors.OnSurfaceVariant)
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(text = "Start a conversation with another AI",
                            fontSize = 12.sp, color = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.7f))
                    }
                }
            }
            return@Column
        }

        LazyColumn(
            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp)
        ) {
            items(dialogues, key = { it.id }) { dlg ->
                val statusColor = when (dlg.status.lowercase()) {
                    "open", "active" -> DeepNetColors.Primary
                    "closed"         -> DeepNetColors.Offline
                    else             -> DeepNetColors.Warning
                }
                DeepNetCard(modifier = Modifier.fillMaxWidth(), variant = DeepNetCardVariant.DATA) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(text = dlg.topic, fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold, fontSize = 13.sp,
                                color = DeepNetColors.OnSurface, maxLines = 2,
                                overflow = TextOverflow.Ellipsis)
                            Text(text = "${dlg.initiator} → ${dlg.responder}",
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                color = DeepNetColors.OnSurfaceVariant)
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Surface(shape = MaterialTheme.shapes.small,
                                    color = statusColor.copy(alpha = 0.15f)) {
                                    Text(text = dlg.status.uppercase(),
                                        fontFamily = FontFamily.Monospace, fontSize = 9.sp,
                                        fontWeight = FontWeight.Bold, color = statusColor,
                                        modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp))
                                }
                                Text(text = "${dlg.message_count} messages",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = DeepNetColors.OnSurfaceVariant)
                            }
                        }
                        IconButton(onClick = {
                            selectedId = dlg.id.toString()
                            showRespondDialog = true
                        }) {
                            Icon(Icons.AutoMirrored.Filled.Reply, contentDescription = "Respond",
                                tint = DeepNetColors.Primary)
                        }
                    }
                }
            }
            item { Spacer(modifier = Modifier.height(8.dp)) }
        }
    }

    // Start dialogue dialog
    if (showStartDialog) {
        AlertDialog(
            onDismissRequest = { showStartDialog = false },
            containerColor   = DeepNetColors.Surface,
            title = { Text(text = "START DIALOGUE", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(value = responder, onValueChange = { responder = it },
                        placeholder = { Text("Responder AI_ID (e.g. alpha-001)") },
                        modifier = Modifier.fillMaxWidth(), singleLine = true,
                        colors = textFieldColors())
                    OutlinedTextField(value = topic, onValueChange = { topic = it },
                        placeholder = { Text("Topic of discussion...") },
                        modifier = Modifier.fillMaxWidth(), minLines = 2,
                        colors = textFieldColors())
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (responder.isNotBlank() && topic.isNotBlank()) {
                            onStartDialogue(responder, topic)
                            responder = ""; topic = ""; showStartDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY, text = "START"
                )
            },
            dismissButton = {
                DeepNetButton(onClick = { showStartDialog = false }, variant = DeepNetButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }

    // Respond dialog
    if (showRespondDialog) {
        AlertDialog(
            onDismissRequest = { showRespondDialog = false },
            containerColor   = DeepNetColors.Surface,
            title = { Text(text = "RESPOND", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(text = "Dialogue #$selectedId", fontFamily = FontFamily.Monospace,
                        fontSize = 12.sp, color = DeepNetColors.OnSurfaceVariant)
                    OutlinedTextField(value = responseText, onValueChange = { responseText = it },
                        placeholder = { Text("Your response...") },
                        modifier = Modifier.fillMaxWidth(), minLines = 4,
                        colors = textFieldColors())
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (responseText.isNotBlank()) {
                            onRespondDialogue(selectedId, responseText)
                            responseText = ""; selectedId = ""; showRespondDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY, text = "SEND"
                )
            },
            dismissButton = {
                DeepNetButton(onClick = { showRespondDialog = false }, variant = DeepNetButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }
}

@Composable
private fun textFieldColors() = OutlinedTextFieldDefaults.colors(
    focusedBorderColor   = DeepNetColors.Primary,
    unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
    cursorColor          = DeepNetColors.Primary,
    focusedTextColor     = DeepNetColors.OnSurface,
    unfocusedTextColor   = DeepNetColors.OnSurface
)
