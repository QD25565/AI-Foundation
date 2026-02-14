package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
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
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors

/**
 * Dialogues screen - Structured AI-to-AI conversations.
 * Start dialogues, respond to invites, view history.
 */
@Composable
fun DialoguesScreen(
    dialoguesData: String,
    onRefresh: () -> Unit,
    onStartDialogue: (String, String) -> Unit, // responder, topic
    onRespondDialogue: (String, String) -> Unit, // id, response
    isLoading: Boolean
) {
    var showStartDialog by remember { mutableStateOf(false) }
    var showRespondDialog by remember { mutableStateOf(false) }
    var responder by remember { mutableStateOf("") }
    var topic by remember { mutableStateOf("") }
    var dialogueId by remember { mutableStateOf("") }
    var responseText by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // Header
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 8.dp)
        ) {
            DeepNetSectionHeader(
                title = "DIALOGUES",
                subtitle = "Structured conversations",
                icon = Icons.Default.Forum
            )
        }

        // Action bar
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            DeepNetButton(
                onClick = onRefresh,
                variant = DeepNetButtonVariant.GHOST,
                icon = Icons.Default.Refresh,
                text = "REFRESH"
            )
            Spacer(modifier = Modifier.weight(1f))
            DeepNetButton(
                onClick = { showRespondDialog = true },
                variant = DeepNetButtonVariant.SECONDARY,
                icon = Icons.Default.Reply,
                text = "RESPOND"
            )
            DeepNetButton(
                onClick = { showStartDialog = true },
                variant = DeepNetButtonVariant.PRIMARY,
                icon = Icons.Default.Add,
                text = "START"
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        if (isLoading) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                DeepNetLoadingIndicator(text = "LOADING DIALOGUES...")
            }
        } else if (dialoguesData.isBlank()) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                DeepNetCard(
                    modifier = Modifier.fillMaxWidth(0.8f),
                    variant = DeepNetCardVariant.TERMINAL
                ) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.fillMaxWidth().padding(24.dp)
                    ) {
                        Icon(
                            imageVector = Icons.Default.Forum,
                            contentDescription = null,
                            tint = DeepNetColors.OnSurfaceVariant,
                            modifier = Modifier.size(48.dp)
                        )
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text = "NO DIALOGUES",
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            color = DeepNetColors.OnSurfaceVariant
                        )
                        Text(
                            text = "Start a conversation with another AI",
                            fontSize = 12.sp,
                            color = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.7f)
                        )
                    }
                }
            }
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                item {
                    DeepNetCard(
                        modifier = Modifier.fillMaxWidth(),
                        variant = DeepNetCardVariant.DATA
                    ) {
                        Text(
                            text = dialoguesData,
                            fontFamily = FontFamily.Monospace,
                            fontSize = 12.sp,
                            color = DeepNetColors.OnSurface,
                            lineHeight = 18.sp
                        )
                    }
                }
            }
        }
    }

    // Start dialogue dialog
    if (showStartDialog) {
        AlertDialog(
            onDismissRequest = { showStartDialog = false },
            containerColor = DeepNetColors.Surface,
            title = {
                Text(
                    text = "START DIALOGUE",
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary
                )
            },
            text = {
                Column {
                    OutlinedTextField(
                        value = responder,
                        onValueChange = { responder = it },
                        placeholder = { Text("Responder AI_ID (e.g. assistant-1)") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor = DeepNetColors.Primary,
                            unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                            cursorColor = DeepNetColors.Primary,
                            focusedTextColor = DeepNetColors.OnSurface,
                            unfocusedTextColor = DeepNetColors.OnSurface
                        )
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    OutlinedTextField(
                        value = topic,
                        onValueChange = { topic = it },
                        placeholder = { Text("Topic of discussion...") },
                        modifier = Modifier.fillMaxWidth(),
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor = DeepNetColors.Primary,
                            unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                            cursorColor = DeepNetColors.Primary,
                            focusedTextColor = DeepNetColors.OnSurface,
                            unfocusedTextColor = DeepNetColors.OnSurface
                        ),
                        minLines = 2
                    )
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (responder.isNotBlank() && topic.isNotBlank()) {
                            onStartDialogue(responder, topic)
                            responder = ""
                            topic = ""
                            showStartDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY,
                    text = "START"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showStartDialog = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text = "CANCEL"
                )
            }
        )
    }

    // Respond to dialogue dialog
    if (showRespondDialog) {
        AlertDialog(
            onDismissRequest = { showRespondDialog = false },
            containerColor = DeepNetColors.Surface,
            title = {
                Text(
                    text = "RESPOND TO DIALOGUE",
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary
                )
            },
            text = {
                Column {
                    OutlinedTextField(
                        value = dialogueId,
                        onValueChange = { dialogueId = it },
                        placeholder = { Text("Dialogue ID") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor = DeepNetColors.Primary,
                            unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                            cursorColor = DeepNetColors.Primary,
                            focusedTextColor = DeepNetColors.OnSurface,
                            unfocusedTextColor = DeepNetColors.OnSurface
                        )
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    OutlinedTextField(
                        value = responseText,
                        onValueChange = { responseText = it },
                        placeholder = { Text("Your response...") },
                        modifier = Modifier.fillMaxWidth(),
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor = DeepNetColors.Primary,
                            unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                            cursorColor = DeepNetColors.Primary,
                            focusedTextColor = DeepNetColors.OnSurface,
                            unfocusedTextColor = DeepNetColors.OnSurface
                        ),
                        minLines = 3
                    )
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (dialogueId.isNotBlank() && responseText.isNotBlank()) {
                            onRespondDialogue(dialogueId, responseText)
                            dialogueId = ""
                            responseText = ""
                            showRespondDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY,
                    text = "SEND"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showRespondDialog = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text = "CANCEL"
                )
            }
        )
    }
}
