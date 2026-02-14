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
 * Inbox screen - DMs and Broadcasts in a tabbed layout.
 */
@Composable
fun InboxScreen(
    dmsData: String,
    broadcastsData: String,
    onRefreshDms: () -> Unit,
    onRefreshBroadcasts: () -> Unit,
    onSendDm: (String, String) -> Unit, // to, content
    onSendBroadcast: (String) -> Unit,
    isLoading: Boolean
) {
    var selectedTab by remember { mutableStateOf(0) }
    var showComposeDialog by remember { mutableStateOf(false) }
    var dmTo by remember { mutableStateOf("") }
    var messageContent by remember { mutableStateOf("") }

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
                title = "INBOX",
                subtitle = "Messages & broadcasts",
                icon = Icons.Default.Inbox
            )
        }

        // Tabs: DMs | Broadcasts
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            DeepNetButton(
                onClick = { selectedTab = 0 },
                variant = if (selectedTab == 0) DeepNetButtonVariant.PRIMARY else DeepNetButtonVariant.GHOST,
                icon = Icons.Default.Mail,
                text = "DMs"
            )
            DeepNetButton(
                onClick = { selectedTab = 1 },
                variant = if (selectedTab == 1) DeepNetButtonVariant.PRIMARY else DeepNetButtonVariant.GHOST,
                icon = Icons.Default.Campaign,
                text = "BROADCASTS"
            )
            Spacer(modifier = Modifier.weight(1f))
            DeepNetButton(
                onClick = { showComposeDialog = true },
                variant = DeepNetButtonVariant.PRIMARY,
                icon = Icons.Default.Edit,
                text = "NEW"
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        if (isLoading) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                DeepNetLoadingIndicator(text = "LOADING...")
            }
        } else {
            val displayData = if (selectedTab == 0) dmsData else broadcastsData

            if (displayData.isBlank()) {
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
                                imageVector = if (selectedTab == 0) Icons.Default.Mail else Icons.Default.Campaign,
                                contentDescription = null,
                                tint = DeepNetColors.OnSurfaceVariant,
                                modifier = Modifier.size(48.dp)
                            )
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text = if (selectedTab == 0) "NO DIRECT MESSAGES" else "NO BROADCASTS",
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold,
                                color = DeepNetColors.OnSurfaceVariant
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
                        // Refresh button
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.End
                        ) {
                            DeepNetButton(
                                onClick = { if (selectedTab == 0) onRefreshDms() else onRefreshBroadcasts() },
                                variant = DeepNetButtonVariant.GHOST,
                                icon = Icons.Default.Refresh,
                                text = "REFRESH"
                            )
                        }
                    }
                    item {
                        DeepNetCard(
                            modifier = Modifier.fillMaxWidth(),
                            variant = DeepNetCardVariant.DATA
                        ) {
                            Text(
                                text = displayData,
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
    }

    // Compose dialog
    if (showComposeDialog) {
        AlertDialog(
            onDismissRequest = { showComposeDialog = false },
            containerColor = DeepNetColors.Surface,
            title = {
                Text(
                    text = if (selectedTab == 0) "SEND DM" else "BROADCAST",
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary
                )
            },
            text = {
                Column {
                    if (selectedTab == 0) {
                        OutlinedTextField(
                            value = dmTo,
                            onValueChange = { dmTo = it },
                            placeholder = { Text("Recipient AI_ID (e.g. assistant-1)") },
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
                    }
                    OutlinedTextField(
                        value = messageContent,
                        onValueChange = { messageContent = it },
                        placeholder = { Text("Message...") },
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
                        if (messageContent.isNotBlank()) {
                            if (selectedTab == 0 && dmTo.isNotBlank()) {
                                onSendDm(dmTo, messageContent)
                            } else if (selectedTab == 1) {
                                onSendBroadcast(messageContent)
                            }
                            messageContent = ""
                            dmTo = ""
                            showComposeDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY,
                    text = "SEND"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showComposeDialog = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text = "CANCEL"
                )
            }
        )
    }
}
