package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Reply
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.Broadcast
import com.aifoundation.app.data.model.Dialogue
import com.aifoundation.app.data.model.Dm
import com.aifoundation.app.data.model.TeamMember
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.AiIdentity
import com.aifoundation.app.ui.theme.DeepNetColors
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * Inbox — three tabs: DMs | Broadcasts | Dialogues.
 *
 * DMs tab shows threaded conversations (grouped by partner, latest first).
 * Tapping a thread or picking a contact opens [ConversationScreen] via [onOpenConversation].
 * SSE pushes real-time updates into [dms] — no manual refresh needed for incoming messages.
 */
@Composable
fun InboxScreen(
    dms: List<Dm>,
    broadcasts: List<Broadcast>,
    dialogues: List<Dialogue>,
    myHId: String,
    team: List<TeamMember>,
    onOpenConversation: (partnerId: String) -> Unit,
    onRefreshDms: () -> Unit,
    onRefreshBroadcasts: () -> Unit,
    onRefreshDialogues: () -> Unit,
    onSendBroadcast: (String) -> Unit,
    onStartDialogue: (String, String) -> Unit,
    onRespondDialogue: (String, String) -> Unit,
    isLoading: Boolean
) {
    var selectedTab by remember { mutableStateOf(0) }

    // New conversation picker dialog state
    var showNewConversation by remember { mutableStateOf(false) }
    var manualRecipient by remember { mutableStateOf("") }

    // Broadcast compose dialog state
    var showBroadcast by remember { mutableStateOf(false) }
    var bcContent by remember { mutableStateOf("") }
    var bcChannel by remember { mutableStateOf("") }

    // Dialogue dialog state
    var showStartDlg   by remember { mutableStateOf(false) }
    var showRespondDlg by remember { mutableStateOf(false) }
    var dlgResponder   by remember { mutableStateOf("") }
    var dlgTopic       by remember { mutableStateOf("") }
    var dlgId          by remember { mutableStateOf("") }
    var dlgResponse    by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // ── Tab bar ───────────────────────────────────────────────────────────
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(DeepNetColors.Surface)
                .padding(horizontal = 8.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment     = Alignment.CenterVertically
        ) {
            TabButton("DMs",        Icons.Default.Mail,     selectedTab == 0) { selectedTab = 0 }
            TabButton("BROADCASTS", Icons.Default.Campaign, selectedTab == 1) { selectedTab = 1 }
            TabButton("DIALOGUES",  Icons.Default.Forum,    selectedTab == 2) { selectedTab = 2 }
            Spacer(modifier = Modifier.weight(1f))
            DeepNetButton(
                onClick = {
                    when (selectedTab) {
                        0 -> showNewConversation = true
                        1 -> showBroadcast       = true
                        2 -> showStartDlg        = true
                    }
                },
                variant = DeepNetButtonVariant.PRIMARY,
                icon    = Icons.Default.Edit,
                text    = "NEW"
            )
        }

        if (isLoading) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetLoadingIndicator(text = "LOADING...")
            }
            return@Column
        }

        // ── Tab content ───────────────────────────────────────────────────────
        when (selectedTab) {
            0 -> DmsTab(
                dms               = dms,
                myHId             = myHId,
                team              = team,
                onOpenConversation = onOpenConversation,
                onRefresh         = onRefreshDms
            )
            1 -> BroadcastsTab(broadcasts = broadcasts, onRefresh = onRefreshBroadcasts)
            2 -> DialoguesTab(
                dialogues = dialogues,
                onRefresh = onRefreshDialogues,
                onRespond = { id ->
                    dlgId = id
                    showRespondDlg = true
                }
            )
        }
    }

    // ── New conversation picker ────────────────────────────────────────────────
    if (showNewConversation) {
        AlertDialog(
            onDismissRequest = { showNewConversation = false; manualRecipient = "" },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(
                    text       = "NEW CONVERSATION",
                    fontFamily = FontFamily.Monospace,
                    color      = DeepNetColors.Primary
                )
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    // Online team members as tappable contact rows
                    val onlineMembers = team.filter { it.online }.sortedBy { it.ai_id }
                    val offlineMembers = team.filter { !it.online }.sortedBy { it.ai_id }

                    if (team.isNotEmpty()) {
                        Text(
                            text       = "TEAM MEMBERS",
                            fontFamily = FontFamily.Monospace,
                            fontSize   = 10.sp,
                            color      = DeepNetColors.OnSurfaceVariant
                        )
                        (onlineMembers + offlineMembers).forEach { member ->
                            ContactPickerRow(
                                member = member,
                                onClick = {
                                    onOpenConversation(member.ai_id)
                                    showNewConversation = false
                                    manualRecipient = ""
                                }
                            )
                        }
                        HorizontalDivider(
                            color     = DeepNetColors.SurfaceVariant,
                            thickness = 1.dp,
                            modifier  = Modifier.padding(vertical = 4.dp)
                        )
                    }

                    // Manual ID entry fallback
                    Text(
                        text       = "OR TYPE AN ID",
                        fontFamily = FontFamily.Monospace,
                        fontSize   = 10.sp,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment     = Alignment.CenterVertically
                    ) {
                        StyledTextField(
                            value         = manualRecipient,
                            onValueChange = { manualRecipient = it },
                            placeholder   = "e.g. alpha-001",
                            modifier      = Modifier.weight(1f)
                        )
                        DeepNetButton(
                            onClick = {
                                val id = manualRecipient.trim()
                                if (id.isNotBlank()) {
                                    onOpenConversation(id)
                                    showNewConversation = false
                                    manualRecipient = ""
                                }
                            },
                            variant = DeepNetButtonVariant.PRIMARY,
                            text    = "OPEN"
                        )
                    }
                }
            },
            confirmButton = {},
            dismissButton = {
                DeepNetButton(
                    onClick = { showNewConversation = false; manualRecipient = "" },
                    variant = DeepNetButtonVariant.GHOST,
                    text    = "CANCEL"
                )
            }
        )
    }

    // ── Broadcast compose dialog ───────────────────────────────────────────────
    if (showBroadcast) {
        AlertDialog(
            onDismissRequest = { showBroadcast = false },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(text = "BROADCAST", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary)
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    StyledTextField(
                        value         = bcChannel,
                        onValueChange = { bcChannel = it },
                        placeholder   = "Channel (optional)"
                    )
                    StyledTextField(
                        value         = bcContent,
                        onValueChange = { bcContent = it },
                        placeholder   = "Message…",
                        minLines      = 3
                    )
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (bcContent.isNotBlank()) {
                            onSendBroadcast(bcContent)
                            bcContent = ""; bcChannel = ""
                            showBroadcast = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY,
                    text    = "SEND"
                )
            },
            dismissButton = {
                DeepNetButton(
                    onClick = { showBroadcast = false },
                    variant = DeepNetButtonVariant.GHOST,
                    text    = "CANCEL"
                )
            }
        )
    }

    // ── Start Dialogue dialog ─────────────────────────────────────────────────
    if (showStartDlg) {
        AlertDialog(
            onDismissRequest = { showStartDlg = false },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(text = "START DIALOGUE", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary)
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    StyledTextField(value = dlgResponder, onValueChange = { dlgResponder = it },
                        placeholder = "Responder AI_ID (e.g. alpha-001)")
                    StyledTextField(value = dlgTopic, onValueChange = { dlgTopic = it },
                        placeholder = "Topic of discussion…", minLines = 2)
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (dlgResponder.isNotBlank() && dlgTopic.isNotBlank()) {
                            onStartDialogue(dlgResponder, dlgTopic)
                            dlgResponder = ""; dlgTopic = ""
                            showStartDlg = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY, text = "START"
                )
            },
            dismissButton = {
                DeepNetButton(onClick = { showStartDlg = false }, variant = DeepNetButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }

    // ── Respond Dialogue dialog ───────────────────────────────────────────────
    if (showRespondDlg) {
        AlertDialog(
            onDismissRequest = { showRespondDlg = false },
            containerColor   = DeepNetColors.Surface,
            title = {
                Text(text = "RESPOND", fontFamily = FontFamily.Monospace, color = DeepNetColors.Primary)
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text       = "Dialogue #$dlgId",
                        fontFamily = FontFamily.Monospace,
                        fontSize   = 12.sp,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                    StyledTextField(value = dlgResponse, onValueChange = { dlgResponse = it },
                        placeholder = "Your response…", minLines = 4)
                }
            },
            confirmButton = {
                DeepNetButton(
                    onClick = {
                        if (dlgResponse.isNotBlank()) {
                            onRespondDialogue(dlgId, dlgResponse)
                            dlgResponse = ""; dlgId = ""
                            showRespondDlg = false
                        }
                    },
                    variant = DeepNetButtonVariant.PRIMARY, text = "SEND"
                )
            },
            dismissButton = {
                DeepNetButton(onClick = { showRespondDlg = false }, variant = DeepNetButtonVariant.GHOST, text = "CANCEL")
            }
        )
    }
}

// ── DMs tab — threaded conversation list ──────────────────────────────────────

@Composable
private fun DmsTab(
    dms: List<Dm>,
    myHId: String,
    team: List<TeamMember>,
    onOpenConversation: (String) -> Unit,
    onRefresh: () -> Unit
) {
    // Group flat DM list into threads: one entry per unique conversation partner.
    // Each thread shows the latest message and is sorted newest-first.
    val threads = remember(dms, myHId) {
        dms.groupBy { dm -> if (dm.from == myHId) dm.to else dm.from }
            .entries
            .map { (partnerId, msgs) ->
                val sorted = msgs.sortedBy { it.timestamp }
                Triple(partnerId, sorted.last(), sorted.size)  // (partnerId, latestDm, count)
            }
            .sortedByDescending { (_, latest, _) -> latest.timestamp }
    }

    if (threads.isEmpty()) {
        EmptyState(icon = Icons.Default.Mail, label = "NO DIRECT MESSAGES")
        return
    }

    LazyColumn(
        contentPadding     = PaddingValues(horizontal = 8.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)
    ) {
        item {
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                DeepNetButton(
                    onClick = onRefresh,
                    variant = DeepNetButtonVariant.GHOST,
                    icon    = Icons.Default.Refresh,
                    text    = "REFRESH"
                )
            }
        }
        items(threads, key = { (partnerId, _, _) -> partnerId }) { (partnerId, latestDm, _) ->
            val partner      = team.find { it.ai_id == partnerId }
            val partnerColor = AiIdentity.colorFor(partnerId)
            val previewText  = if (latestDm.from == myHId) "You: ${latestDm.content}" else latestDm.content

            DeepNetCard(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { onOpenConversation(partnerId) },
                variant  = DeepNetCardVariant.DATA
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    // Online presence dot
                    Box(
                        modifier = Modifier
                            .size(8.dp)
                            .clip(CircleShape)
                            .background(
                                if (partner?.online == true) DeepNetColors.Online
                                else DeepNetColors.Offline.copy(alpha = 0.4f)
                            )
                    )

                    // Colored identity avatar
                    Box(
                        modifier = Modifier
                            .size(44.dp)
                            .clip(CircleShape)
                            .background(AiIdentity.avatarBackground(partnerId))
                            .border(1.5.dp, AiIdentity.avatarBorder(partnerId), CircleShape),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            text       = AiIdentity.initial(partnerId),
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            fontSize   = 16.sp,
                            color      = partnerColor
                        )
                    }

                    // Partner name + last message preview
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text       = partnerId,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            fontSize   = 14.sp,
                            color      = DeepNetColors.OnSurface
                        )
                        Spacer(modifier = Modifier.height(2.dp))
                        Text(
                            text     = previewText,
                            style    = MaterialTheme.typography.bodySmall,
                            color    = DeepNetColors.OnSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    }

                    // Relative timestamp
                    Column(horizontalAlignment = Alignment.End) {
                        Text(
                            text       = formatThreadTime(latestDm.timestamp),
                            style      = MaterialTheme.typography.labelSmall,
                            fontFamily = FontFamily.Monospace,
                            color      = DeepNetColors.OnSurfaceVariant
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        Icon(
                            imageVector        = Icons.Default.ChevronRight,
                            contentDescription = null,
                            tint               = DeepNetColors.OnSurfaceVariant,
                            modifier           = Modifier.size(16.dp)
                        )
                    }
                }
            }
        }
    }
}

// ── Broadcasts tab ────────────────────────────────────────────────────────────

@Composable
private fun BroadcastsTab(broadcasts: List<Broadcast>, onRefresh: () -> Unit) {
    if (broadcasts.isEmpty()) {
        EmptyState(icon = Icons.Default.Campaign, label = "NO BROADCASTS")
        return
    }
    LazyColumn(
        contentPadding     = PaddingValues(horizontal = 8.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)
    ) {
        item {
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                DeepNetButton(onClick = onRefresh, variant = DeepNetButtonVariant.GHOST,
                    icon = Icons.Default.Refresh, text = "REFRESH")
            }
        }
        items(broadcasts, key = { it.id }) { bc ->
            DeepNetCard(modifier = Modifier.fillMaxWidth(), variant = DeepNetCardVariant.DATA) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment     = Alignment.Top
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                            verticalAlignment     = Alignment.CenterVertically
                        ) {
                            // Colored sender avatar
                            Box(
                                modifier = Modifier
                                    .size(24.dp)
                                    .clip(CircleShape)
                                    .background(AiIdentity.avatarBackground(bc.from)),
                                contentAlignment = Alignment.Center
                            ) {
                                Text(
                                    text       = AiIdentity.initial(bc.from),
                                    fontFamily = FontFamily.Monospace,
                                    fontWeight = FontWeight.Bold,
                                    fontSize   = 10.sp,
                                    color      = AiIdentity.colorFor(bc.from)
                                )
                            }
                            Text(
                                text       = bc.from,
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold,
                                fontSize   = 13.sp,
                                color      = AiIdentity.colorFor(bc.from)
                            )
                            if (bc.channel.isNotBlank() && bc.channel != "general") {
                                Surface(
                                    shape = MaterialTheme.shapes.small,
                                    color = DeepNetColors.Primary.copy(alpha = 0.12f)
                                ) {
                                    Text(
                                        text       = "#${bc.channel}",
                                        fontFamily = FontFamily.Monospace,
                                        fontSize   = 9.sp,
                                        color      = DeepNetColors.Primary,
                                        modifier   = Modifier.padding(horizontal = 5.dp, vertical = 2.dp)
                                    )
                                }
                            }
                        }
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text     = bc.content,
                            style    = MaterialTheme.typography.bodySmall,
                            color    = DeepNetColors.OnSurface,
                            maxLines = 4,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        text       = bc.timestamp.take(16),
                        style      = MaterialTheme.typography.labelSmall,
                        color      = DeepNetColors.OnSurfaceVariant,
                        fontFamily = FontFamily.Monospace
                    )
                }
            }
        }
    }
}

// ── Dialogues tab ─────────────────────────────────────────────────────────────

@Composable
private fun DialoguesTab(
    dialogues: List<Dialogue>,
    onRefresh: () -> Unit,
    onRespond: (String) -> Unit
) {
    if (dialogues.isEmpty()) {
        EmptyState(icon = Icons.Default.Forum, label = "NO DIALOGUES")
        return
    }
    LazyColumn(
        contentPadding     = PaddingValues(horizontal = 8.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)
    ) {
        item {
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                DeepNetButton(onClick = onRefresh, variant = DeepNetButtonVariant.GHOST,
                    icon = Icons.Default.Refresh, text = "REFRESH")
            }
        }
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
                    verticalAlignment     = Alignment.Top
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text       = dlg.topic,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            fontSize   = 13.sp,
                            color      = DeepNetColors.OnSurface,
                            maxLines   = 2,
                            overflow   = TextOverflow.Ellipsis
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                            Box(
                                modifier = Modifier
                                    .size(18.dp)
                                    .clip(CircleShape)
                                    .background(AiIdentity.avatarBackground(dlg.initiator)),
                                contentAlignment = Alignment.Center
                            ) {
                                Text(AiIdentity.initial(dlg.initiator), fontSize = 8.sp,
                                    fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold,
                                    color = AiIdentity.colorFor(dlg.initiator))
                            }
                            Text(
                                text       = "${dlg.initiator} → ${dlg.responder}",
                                style      = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                color      = DeepNetColors.OnSurfaceVariant
                            )
                        }
                        Spacer(modifier = Modifier.height(6.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            StatusChip(label = dlg.status.uppercase(), color = statusColor)
                            Text(
                                text  = "${dlg.message_count} messages",
                                style = MaterialTheme.typography.labelSmall,
                                color = DeepNetColors.OnSurfaceVariant
                            )
                        }
                    }
                    IconButton(onClick = { onRespond(dlg.id.toString()) }) {
                        Icon(
                            imageVector        = Icons.AutoMirrored.Filled.Reply,
                            contentDescription = "Respond",
                            tint               = DeepNetColors.Primary
                        )
                    }
                }
            }
        }
    }
}

// ── Shared composables ────────────────────────────────────────────────────────

@Composable
private fun ContactPickerRow(member: TeamMember, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 6.dp, horizontal = 4.dp),
        verticalAlignment     = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        // Online dot
        Box(
            modifier = Modifier
                .size(7.dp)
                .clip(CircleShape)
                .background(if (member.online) DeepNetColors.Online else DeepNetColors.Offline.copy(alpha = 0.4f))
        )
        // Avatar
        Box(
            modifier = Modifier
                .size(30.dp)
                .clip(CircleShape)
                .background(AiIdentity.avatarBackground(member.ai_id)),
            contentAlignment = Alignment.Center
        ) {
            Text(
                text       = AiIdentity.initial(member.ai_id),
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize   = 12.sp,
                color      = AiIdentity.colorFor(member.ai_id)
            )
        }
        // Name
        Text(
            text       = member.ai_id,
            fontFamily = FontFamily.Monospace,
            fontSize   = 13.sp,
            fontWeight = FontWeight.Medium,
            color      = DeepNetColors.OnSurface,
            modifier   = Modifier.weight(1f)
        )
        // Status
        if (member.online) {
            Text(
                text   = "online",
                style  = MaterialTheme.typography.labelSmall,
                color  = DeepNetColors.Online,
                fontFamily = FontFamily.Monospace
            )
        }
    }
}

@Composable
private fun TabButton(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    selected: Boolean,
    onClick: () -> Unit
) {
    DeepNetButton(
        onClick = onClick,
        variant = if (selected) DeepNetButtonVariant.PRIMARY else DeepNetButtonVariant.GHOST,
        icon    = icon,
        text    = label
    )
}

@Composable
private fun StyledTextField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    minLines: Int = 1,
    modifier: Modifier = Modifier
) {
    OutlinedTextField(
        value         = value,
        onValueChange = onValueChange,
        placeholder   = { Text(placeholder, fontSize = 13.sp) },
        modifier      = modifier.fillMaxWidth(),
        minLines      = minLines,
        colors        = OutlinedTextFieldDefaults.colors(
            focusedBorderColor   = DeepNetColors.Primary,
            unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
            cursorColor          = DeepNetColors.Primary,
            focusedTextColor     = DeepNetColors.OnSurface,
            unfocusedTextColor   = DeepNetColors.OnSurface
        )
    )
}

@Composable
private fun StatusChip(label: String, color: androidx.compose.ui.graphics.Color) {
    Surface(shape = MaterialTheme.shapes.small, color = color.copy(alpha = 0.15f)) {
        Text(
            text       = label,
            fontFamily = FontFamily.Monospace,
            fontSize   = 9.sp,
            fontWeight = FontWeight.Bold,
            color      = color,
            modifier   = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
        )
    }
}

@Composable
private fun EmptyState(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        DeepNetCard(modifier = Modifier.fillMaxWidth(0.75f), variant = DeepNetCardVariant.TERMINAL) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.fillMaxWidth().padding(32.dp)
            ) {
                Icon(imageVector = icon, contentDescription = null,
                    tint = DeepNetColors.OnSurfaceVariant, modifier = Modifier.size(48.dp))
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text       = label,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color      = DeepNetColors.OnSurfaceVariant
                )
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Formats a thread's last-message timestamp for the thread list:
 * same-day → "HH:mm", different day → "MMM d".
 */
private fun formatThreadTime(timestamp: String): String = try {
    val zoned = Instant.parse(timestamp).atZone(ZoneId.systemDefault())
    val today = LocalDate.now(ZoneId.systemDefault())
    if (zoned.toLocalDate() == today) {
        zoned.format(DateTimeFormatter.ofPattern("HH:mm"))
    } else {
        zoned.format(DateTimeFormatter.ofPattern("MMM d"))
    }
} catch (_: Exception) {
    timestamp.take(10)
}
