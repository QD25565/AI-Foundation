package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.automirrored.filled.MenuBook
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
import com.aifoundation.app.data.model.Note
import com.aifoundation.app.data.model.NoteSearchResult
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.FoundationColors
import kotlin.math.roundToInt

/**
 * Notebook screen — personal memory with semantic search.
 * Note cards show content + tag chips + pinned star.
 * Search results show relevance score.
 */
@Composable
fun NotebookScreen(
    notes: List<Note>,
    noteSearchResults: List<NoteSearchResult>,
    onRefresh: () -> Unit,
    onRemember: (String, String?) -> Unit,
    onRecall: (String) -> Unit,
    isLoading: Boolean
) {
    var showRememberDialog by remember { mutableStateOf(false) }
    var noteContent        by remember { mutableStateOf("") }
    var noteTags           by remember { mutableStateOf("") }
    var searchQuery        by remember { mutableStateOf("") }
    var isSearchMode       by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(FoundationColors.Background)
    ) {
        // Header
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column {
                Text(
                    text       = "NOTEBOOK",
                    style      = MaterialTheme.typography.headlineSmall,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Black,
                    color      = FoundationColors.Primary
                )
                if (notes.isNotEmpty() && !isSearchMode) {
                    Text(
                        text       = "${notes.size} notes · ${notes.count { it.pinned }} pinned",
                        style      = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color      = FoundationColors.OnSurfaceVariant
                    )
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (isSearchMode) {
                    FoundationButton(
                        onClick = { isSearchMode = false; searchQuery = ""; onRefresh() },
                        variant = FoundationButtonVariant.GHOST,
                        icon    = Icons.AutoMirrored.Filled.List,
                        text    = "RECENT"
                    )
                }
                FoundationButton(
                    onClick = { showRememberDialog = true },
                    variant = FoundationButtonVariant.PRIMARY,
                    icon    = Icons.Default.Add,
                    text    = "NOTE"
                )
            }
        }

        // Search bar
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            OutlinedTextField(
                value         = searchQuery,
                onValueChange = { searchQuery = it },
                placeholder   = { Text("Semantic search...", fontSize = 13.sp) },
                modifier      = Modifier.weight(1f),
                singleLine    = true,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor   = FoundationColors.Primary,
                    unfocusedBorderColor = FoundationColors.OnSurfaceVariant.copy(alpha = 0.4f),
                    cursorColor          = FoundationColors.Primary,
                    focusedTextColor     = FoundationColors.OnSurface,
                    unfocusedTextColor   = FoundationColors.OnSurface
                ),
                trailingIcon = {
                    if (searchQuery.isNotBlank()) {
                        IconButton(onClick = {
                            onRecall(searchQuery)
                            isSearchMode = true
                        }) {
                            Icon(Icons.Default.Search, contentDescription = "Search",
                                tint = FoundationColors.Primary)
                        }
                    }
                }
            )
        }

        if (isSearchMode && searchQuery.isNotBlank()) {
            Text(
                text       = "Results for \"$searchQuery\"",
                style      = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color      = FoundationColors.Primary,
                modifier   = Modifier.padding(horizontal = 12.dp, vertical = 2.dp)
            )
        }

        if (isLoading) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                FoundationLoadingIndicator(text = "LOADING NOTES...")
            }
            return@Column
        }

        if (isSearchMode) {
            // Search results view
            if (noteSearchResults.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        text       = "No results for \"$searchQuery\"",
                        fontFamily = FontFamily.Monospace,
                        color      = FoundationColors.OnSurfaceVariant
                    )
                }
            } else {
                LazyColumn(
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    items(noteSearchResults, key = { it.id }) { result ->
                        SearchResultCard(result = result)
                    }
                    item { Spacer(modifier = Modifier.height(8.dp)) }
                }
            }
        } else {
            // Recent notes view
            if (notes.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    FoundationCard(modifier = Modifier.fillMaxWidth(0.75f), variant = FoundationCardVariant.TERMINAL) {
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            modifier = Modifier.fillMaxWidth().padding(32.dp)
                        ) {
                            Icon(imageVector = Icons.AutoMirrored.Filled.MenuBook, contentDescription = null,
                                tint = FoundationColors.OnSurfaceVariant, modifier = Modifier.size(48.dp))
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text       = "NO NOTES YET",
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold,
                                color      = FoundationColors.OnSurfaceVariant
                            )
                            Spacer(modifier = Modifier.height(4.dp))
                            Text(
                                text  = "Tap + to remember something",
                                fontSize = 12.sp,
                                color = FoundationColors.OnSurfaceVariant.copy(alpha = 0.7f)
                            )
                        }
                    }
                }
            } else {
                LazyColumn(
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    item {
                        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                            FoundationButton(onClick = onRefresh, variant = FoundationButtonVariant.GHOST,
                                icon = Icons.Default.Refresh, text = "REFRESH")
                        }
                    }
                    items(notes, key = { it.id }) { note ->
                        NoteCard(note = note)
                    }
                    item { Spacer(modifier = Modifier.height(8.dp)) }
                }
            }
        }
    }

    // ── Remember dialog ───────────────────────────────────────────────────────
    if (showRememberDialog) {
        AlertDialog(
            onDismissRequest = { showRememberDialog = false },
            containerColor   = FoundationColors.Surface,
            title = {
                Text(text = "REMEMBER", fontFamily = FontFamily.Monospace, color = FoundationColors.Primary)
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(
                        value         = noteContent,
                        onValueChange = { noteContent = it },
                        placeholder   = { Text("What to remember...") },
                        modifier      = Modifier.fillMaxWidth(),
                        minLines      = 3,
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor   = FoundationColors.Primary,
                            unfocusedBorderColor = FoundationColors.OnSurfaceVariant,
                            cursorColor          = FoundationColors.Primary,
                            focusedTextColor     = FoundationColors.OnSurface,
                            unfocusedTextColor   = FoundationColors.OnSurface
                        )
                    )
                    OutlinedTextField(
                        value         = noteTags,
                        onValueChange = { noteTags = it },
                        placeholder   = { Text("Tags (comma-separated, optional)") },
                        modifier      = Modifier.fillMaxWidth(),
                        singleLine    = true,
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedBorderColor   = FoundationColors.Secondary,
                            unfocusedBorderColor = FoundationColors.OnSurfaceVariant,
                            cursorColor          = FoundationColors.Primary,
                            focusedTextColor     = FoundationColors.OnSurface,
                            unfocusedTextColor   = FoundationColors.OnSurface
                        )
                    )
                }
            },
            confirmButton = {
                FoundationButton(
                    onClick = {
                        if (noteContent.isNotBlank()) {
                            onRemember(noteContent, noteTags.ifBlank { null })
                            noteContent = ""; noteTags = ""
                            showRememberDialog = false
                        }
                    },
                    variant = FoundationButtonVariant.PRIMARY, text = "SAVE"
                )
            },
            dismissButton = {
                FoundationButton(onClick = { showRememberDialog = false }, variant = FoundationButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }
}

@Composable
private fun NoteCard(note: Note) {
    FoundationCard(
        modifier   = Modifier.fillMaxWidth(),
        variant    = FoundationCardVariant.DATA,
        enableGlow = note.pinned
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            // Content preview
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.Top
            ) {
                Text(
                    text     = note.content,
                    style    = MaterialTheme.typography.bodySmall,
                    color    = FoundationColors.OnSurface,
                    maxLines = 4,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f)
                )
                if (note.pinned) {
                    Spacer(modifier = Modifier.width(8.dp))
                    Icon(
                        imageVector = Icons.Default.PushPin,
                        contentDescription = "Pinned",
                        tint = FoundationColors.Warning,
                        modifier = Modifier.size(14.dp)
                    )
                }
            }

            // Tags + date
            if (note.tags.isNotEmpty() || note.created_at.isNotEmpty()) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    // Tag chips
                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        note.tags.take(4).forEach { tag ->
                            Surface(
                                shape = MaterialTheme.shapes.small,
                                color = FoundationColors.SurfaceVariant
                            ) {
                                Text(
                                    text       = tag,
                                    fontFamily = FontFamily.Monospace,
                                    fontSize   = 9.sp,
                                    color      = FoundationColors.OnSurfaceVariant,
                                    modifier   = Modifier.padding(horizontal = 5.dp, vertical = 2.dp)
                                )
                            }
                        }
                        if (note.tags.size > 4) {
                            Text(
                                text   = "+${note.tags.size - 4}",
                                style  = MaterialTheme.typography.labelSmall,
                                color  = FoundationColors.OnSurfaceVariant
                            )
                        }
                    }
                    Text(
                        text       = note.created_at.take(10),
                        fontFamily = FontFamily.Monospace,
                        fontSize   = 10.sp,
                        color      = FoundationColors.OnSurfaceVariant.copy(alpha = 0.6f)
                    )
                }
            }
        }
    }
}

@Composable
private fun SearchResultCard(result: NoteSearchResult) {
    val scorePct = (result.score * 100).roundToInt()
    FoundationCard(modifier = Modifier.fillMaxWidth(), variant = FoundationCardVariant.NODE) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.Top
            ) {
                Text(
                    text     = result.content,
                    style    = MaterialTheme.typography.bodySmall,
                    color    = FoundationColors.OnSurface,
                    maxLines = 4,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f)
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text       = "$scorePct%",
                    fontFamily = FontFamily.Monospace,
                    fontSize   = 11.sp,
                    fontWeight = FontWeight.Bold,
                    color      = when {
                        scorePct >= 80 -> FoundationColors.Online
                        scorePct >= 50 -> FoundationColors.Warning
                        else           -> FoundationColors.OnSurfaceVariant
                    }
                )
            }
            if (result.tags.isNotEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    result.tags.take(4).forEach { tag ->
                        Surface(shape = MaterialTheme.shapes.small, color = FoundationColors.SurfaceVariant) {
                            Text(
                                text       = tag,
                                fontFamily = FontFamily.Monospace,
                                fontSize   = 9.sp,
                                color      = FoundationColors.OnSurfaceVariant,
                                modifier   = Modifier.padding(horizontal = 5.dp, vertical = 2.dp)
                            )
                        }
                    }
                }
            }
        }
    }
}
