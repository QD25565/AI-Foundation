package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.Task
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors

/**
 * Tasks screen — shared task queue with typed cards.
 * Tap a task to update its status. New tasks via the button.
 */
@Composable
fun TasksScreen(
    tasks: List<Task>,
    onRefresh: () -> Unit,
    onCreateTask: (String) -> Unit,
    onUpdateTask: (String, String) -> Unit,   // id, status
    isLoading: Boolean
) {
    var showCreateDialog by remember { mutableStateOf(false) }
    var showUpdateDialog by remember { mutableStateOf(false) }
    var newTaskDesc      by remember { mutableStateOf("") }
    var selectedTask     by remember { mutableStateOf<Task?>(null) }
    var newStatus        by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // Header row
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column {
                Text(
                    text       = "TASKS",
                    style      = MaterialTheme.typography.headlineSmall,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Black,
                    color      = DeepNetColors.Primary
                )
                if (tasks.isNotEmpty()) {
                    val pending = tasks.count { it.status == "pending" }
                    val active  = tasks.count { it.status in listOf("claimed","started","in_progress","in-progress") }
                    Text(
                        text       = "$pending pending · $active active · ${tasks.size} total",
                        style      = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                DeepNetButton(
                    onClick = onRefresh,
                    variant = DeepNetButtonVariant.GHOST,
                    icon    = Icons.Default.Refresh,
                    text    = "REFRESH"
                )
                DeepNetButton(
                    onClick = { showCreateDialog = true },
                    variant = DeepNetButtonVariant.PRIMARY,
                    icon    = Icons.Default.Add,
                    text    = "NEW"
                )
            }
        }

        if (isLoading) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetLoadingIndicator(text = "LOADING TASKS...")
            }
            return@Column
        }

        if (tasks.isEmpty()) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetCard(modifier = Modifier.fillMaxWidth(0.75f), variant = DeepNetCardVariant.TERMINAL) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.fillMaxWidth().padding(32.dp)
                    ) {
                        Icon(imageVector = Icons.Default.TaskAlt, contentDescription = null,
                            tint = DeepNetColors.OnSurfaceVariant, modifier = Modifier.size(48.dp))
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text       = "NO TASKS",
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            color      = DeepNetColors.OnSurfaceVariant
                        )
                    }
                }
            }
            return@Column
        }

        LazyColumn(
            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp)
        ) {
            items(tasks, key = { it.id }) { task ->
                TaskCard(
                    task   = task,
                    onTap  = {
                        selectedTask = task
                        newStatus    = task.status
                        showUpdateDialog = true
                    }
                )
            }
            item { Spacer(modifier = Modifier.height(8.dp)) }
        }
    }

    // ── Create task dialog ────────────────────────────────────────────────────
    if (showCreateDialog) {
        AlertDialog(
            onDismissRequest = { showCreateDialog = false },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(text = "NEW TASK", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary)
            },
            text = {
                OutlinedTextField(
                    value         = newTaskDesc,
                    onValueChange = { newTaskDesc = it },
                    placeholder   = { Text("Task description...") },
                    modifier      = Modifier.fillMaxWidth(),
                    minLines      = 2,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor   = DeepNetColors.Primary,
                        unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                        cursorColor          = DeepNetColors.Primary,
                        focusedTextColor     = DeepNetColors.OnSurface,
                        unfocusedTextColor   = DeepNetColors.OnSurface
                    )
                )
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (newTaskDesc.isNotBlank()) {
                            onCreateTask(newTaskDesc)
                            newTaskDesc = ""
                            showCreateDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY, text = "CREATE"
                )
            },
            dismissButton = {
                DeepNetButton(onClick = { showCreateDialog = false }, variant = DeepNetButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }

    // ── Update task status dialog ─────────────────────────────────────────────
    if (showUpdateDialog && selectedTask != null) {
        val task = selectedTask!!
        AlertDialog(
            onDismissRequest = { showUpdateDialog = false },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(text = "UPDATE TASK", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary)
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text       = task.description,
                        style      = MaterialTheme.typography.bodyMedium,
                        color      = DeepNetColors.OnSurface,
                        maxLines   = 3,
                        overflow   = TextOverflow.Ellipsis
                    )
                    Text(
                        text       = "Set status:",
                        style      = MaterialTheme.typography.labelMedium,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                    // Status options
                    listOf("pending", "started", "done", "blocked", "closed").forEach { status ->
                        val (color, _) = taskStatusStyle(status)
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { newStatus = status }
                                .padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp)
                        ) {
                            RadioButton(
                                selected = newStatus == status,
                                onClick  = { newStatus = status },
                                colors   = RadioButtonDefaults.colors(selectedColor = color)
                            )
                            Text(
                                text       = status.uppercase(),
                                fontFamily = FontFamily.Monospace,
                                fontSize   = 13.sp,
                                color      = color
                            )
                        }
                    }
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (newStatus.isNotBlank()) {
                            onUpdateTask(task.id, newStatus)
                            showUpdateDialog = false
                            selectedTask = null
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY, text = "UPDATE"
                )
            },
            dismissButton = {
                DeepNetButton(onClick = { showUpdateDialog = false }, variant = DeepNetButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }
}

@Composable
private fun TaskCard(task: Task, onTap: () -> Unit) {
    val (statusColor, statusLabel) = taskStatusStyle(task.status)

    DeepNetCard(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onTap),
        variant  = DeepNetCardVariant.DATA
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            // Description
            Text(
                text     = task.description,
                style    = MaterialTheme.typography.bodyMedium,
                color    = DeepNetColors.OnSurface,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis
            )

            // Meta row
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                // Status badge
                Surface(
                    shape = MaterialTheme.shapes.small,
                    color = statusColor.copy(alpha = 0.15f)
                ) {
                    Text(
                        text       = statusLabel,
                        fontFamily = FontFamily.Monospace,
                        fontSize   = 9.sp,
                        fontWeight = FontWeight.Bold,
                        color      = statusColor,
                        modifier   = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
                    )
                }

                // Owner
                task.owner?.let { owner ->
                    Text(
                        text       = owner,
                        fontFamily = FontFamily.Monospace,
                        fontSize   = 11.sp,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                }

                Spacer(modifier = Modifier.weight(1f))

                // ID
                Text(
                    text       = "#${task.id.take(8)}",
                    fontFamily = FontFamily.Monospace,
                    fontSize   = 10.sp,
                    color      = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.6f)
                )
            }
        }
    }
}

private fun taskStatusStyle(status: String): Pair<Color, String> = when (status.lowercase()) {
    "pending"                                   -> DeepNetColors.OnSurfaceVariant to "PENDING"
    "claimed", "started", "in_progress",
    "in-progress"                               -> DeepNetColors.Warning to "IN PROGRESS"
    "done", "completed"                         -> DeepNetColors.Online  to "DONE"
    "blocked"                                   -> DeepNetColors.Error   to "BLOCKED"
    "closed"                                    -> DeepNetColors.Offline to "CLOSED"
    else                                        -> DeepNetColors.Secondary to status.uppercase()
}
