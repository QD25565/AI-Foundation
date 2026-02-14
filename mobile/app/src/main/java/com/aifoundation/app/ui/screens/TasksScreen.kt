package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
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
 * Tasks screen - View and manage shared tasks.
 * Data comes as CLI text output from the teambook HTTP API.
 */
@Composable
fun TasksScreen(
    tasksData: String,
    onRefresh: () -> Unit,
    onCreateTask: (String) -> Unit,
    onUpdateTask: (String, String) -> Unit, // id, status
    isLoading: Boolean
) {
    var showCreateDialog by remember { mutableStateOf(false) }
    var newTaskDescription by remember { mutableStateOf("") }

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
                title = "TASKS",
                subtitle = "Shared task queue",
                icon = Icons.Default.TaskAlt
            )
        }

        // Action bar
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            DeepNetButton(
                onClick = onRefresh,
                variant = DeepNetButtonVariant.GHOST,
                icon = Icons.Default.Refresh,
                text = "REFRESH"
            )
            DeepNetButton(
                onClick = { showCreateDialog = true },
                variant = DeepNetButtonVariant.PRIMARY,
                icon = Icons.Default.Add,
                text = "NEW TASK"
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        if (isLoading) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                DeepNetLoadingIndicator(text = "LOADING TASKS...")
            }
        } else if (tasksData.isBlank()) {
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
                            imageVector = Icons.Default.TaskAlt,
                            contentDescription = null,
                            tint = DeepNetColors.OnSurfaceVariant,
                            modifier = Modifier.size(48.dp)
                        )
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text = "NO TASKS",
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            color = DeepNetColors.OnSurfaceVariant
                        )
                    }
                }
            }
        } else {
            // Parse and display tasks as CLI output
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                // Display the raw CLI output in a styled card
                item {
                    DeepNetCard(
                        modifier = Modifier.fillMaxWidth(),
                        variant = DeepNetCardVariant.DATA
                    ) {
                        Text(
                            text = tasksData,
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

    // Create task dialog
    if (showCreateDialog) {
        AlertDialog(
            onDismissRequest = { showCreateDialog = false },
            containerColor = DeepNetColors.Surface,
            title = {
                Text(
                    text = "NEW TASK",
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary
                )
            },
            text = {
                OutlinedTextField(
                    value = newTaskDescription,
                    onValueChange = { newTaskDescription = it },
                    placeholder = { Text("Task description...") },
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
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (newTaskDescription.isNotBlank()) {
                            onCreateTask(newTaskDescription)
                            newTaskDescription = ""
                            showCreateDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY,
                    text = "CREATE"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showCreateDialog = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text = "CANCEL"
                )
            }
        )
    }
}
