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
 * Notebook screen - Personal notes with semantic search.
 * Remember new notes, recall by query, list recent.
 */
@Composable
fun NotebookScreen(
    notesData: String,
    searchResults: String,
    onRefresh: () -> Unit,
    onRemember: (String, String?) -> Unit, // content, tags
    onRecall: (String) -> Unit,
    isLoading: Boolean
) {
    var showRememberDialog by remember { mutableStateOf(false) }
    var noteContent by remember { mutableStateOf("") }
    var noteTags by remember { mutableStateOf("") }
    var searchQuery by remember { mutableStateOf("") }
    var isSearchMode by remember { mutableStateOf(false) }

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
                title = "NOTEBOOK",
                subtitle = "Personal memory",
                icon = Icons.Default.MenuBook
            )
        }

        // Search bar
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            OutlinedTextField(
                value = searchQuery,
                onValueChange = { searchQuery = it },
                placeholder = { Text("Search notes...", fontSize = 13.sp) },
                modifier = Modifier.weight(1f),
                singleLine = true,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = DeepNetColors.Primary,
                    unfocusedBorderColor = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.5f),
                    cursorColor = DeepNetColors.Primary,
                    focusedTextColor = DeepNetColors.OnSurface,
                    unfocusedTextColor = DeepNetColors.OnSurface
                ),
                trailingIcon = {
                    if (searchQuery.isNotBlank()) {
                        IconButton(onClick = {
                            onRecall(searchQuery)
                            isSearchMode = true
                        }) {
                            Icon(
                                Icons.Default.Search,
                                contentDescription = "Search",
                                tint = DeepNetColors.Primary
                            )
                        }
                    }
                }
            )
            DeepNetButton(
                onClick = { showRememberDialog = true },
                variant = DeepNetButtonVariant.PRIMARY,
                icon = Icons.Default.Add,
                text = "NOTE"
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Toggle: Recent vs Search Results
        if (isSearchMode && searchResults.isNotBlank()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                DeepNetButton(
                    onClick = {
                        isSearchMode = false
                        onRefresh()
                    },
                    variant = DeepNetButtonVariant.GHOST,
                    icon = Icons.Default.List,
                    text = "RECENT"
                )
                Text(
                    text = "Search: \"$searchQuery\"",
                    fontSize = 12.sp,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary,
                    modifier = Modifier.align(Alignment.CenterVertically)
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
        }

        if (isLoading) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                DeepNetLoadingIndicator(text = "LOADING NOTES...")
            }
        } else {
            val displayData = if (isSearchMode && searchResults.isNotBlank()) searchResults else notesData

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
                                imageVector = Icons.Default.MenuBook,
                                contentDescription = null,
                                tint = DeepNetColors.OnSurfaceVariant,
                                modifier = Modifier.size(48.dp)
                            )
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text = "NO NOTES YET",
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold,
                                color = DeepNetColors.OnSurfaceVariant
                            )
                            Text(
                                text = "Tap + to remember something",
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

    // Remember dialog
    if (showRememberDialog) {
        AlertDialog(
            onDismissRequest = { showRememberDialog = false },
            containerColor = DeepNetColors.Surface,
            title = {
                Text(
                    text = "REMEMBER",
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary
                )
            },
            text = {
                Column {
                    OutlinedTextField(
                        value = noteContent,
                        onValueChange = { noteContent = it },
                        placeholder = { Text("What to remember...") },
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
                    Spacer(modifier = Modifier.height(8.dp))
                    OutlinedTextField(
                        value = noteTags,
                        onValueChange = { noteTags = it },
                        placeholder = { Text("Tags (comma-separated)") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor = DeepNetColors.Secondary,
                            unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                            cursorColor = DeepNetColors.Primary,
                            focusedTextColor = DeepNetColors.OnSurface,
                            unfocusedTextColor = DeepNetColors.OnSurface
                        )
                    )
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (noteContent.isNotBlank()) {
                            onRemember(noteContent, noteTags.ifBlank { null })
                            noteContent = ""
                            noteTags = ""
                            showRememberDialog = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY,
                    text = "SAVE"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showRememberDialog = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text = "CANCEL"
                )
            }
        )
    }
}
