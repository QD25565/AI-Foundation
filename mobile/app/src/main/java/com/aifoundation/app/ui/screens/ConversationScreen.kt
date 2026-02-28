package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.Dm
import com.aifoundation.app.data.model.TeamMember
import com.aifoundation.app.ui.theme.AiIdentity
import com.aifoundation.app.ui.theme.FoundationColors
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * Full-screen conversation between the authenticated human and a single partner.
 *
 * Messages are filtered client-side from the global DM list — SSE pushes new incoming
 * DMs into that list and Compose recomposition propagates them here automatically,
 * with no polling or extra API calls needed.
 *
 * Layout:
 *   ┌─ Header: back ← [avatar] partner-name  ● online ─────┐
 *   │                                                        │
 *   │      [date separator: TODAY]                           │
 *   │                                Sent bubble ░░░░░░░░░  │
 *   │  ░░░░░░░░░ Received bubble                             │
 *   │                                                        │
 *   └─ [type a message…]                          [▶ SEND] ─┘
 */
@Composable
fun ConversationScreen(
    partnerId: String,
    myHId: String,
    allDms: List<Dm>,
    team: List<TeamMember>,
    onSendDm: (to: String, content: String) -> Unit,
    onBack: () -> Unit
) {
    val partner      = remember(team, partnerId) { team.find { it.ai_id == partnerId } }
    val partnerColor = remember(partnerId) { AiIdentity.colorFor(partnerId) }

    // Filter to this conversation, sorted oldest → newest (ascending for chat layout).
    val messages = remember(allDms, partnerId, myHId) {
        allDms
            .filter { (it.from == myHId && it.to == partnerId) || (it.from == partnerId && it.to == myHId) }
            .sortedBy { it.timestamp }
    }

    // Pre-compute display items (date separators, bubble sides, formatted timestamps).
    val messageItems = remember(messages, myHId) { buildMessageItems(messages, myHId) }

    var inputText by remember { mutableStateOf("") }
    val listState = rememberLazyListState()
    val isFirstScroll = remember { mutableStateOf(true) }

    // Jump to bottom on load; animate scroll when new messages arrive via SSE.
    LaunchedEffect(messageItems.size) {
        if (messageItems.isNotEmpty()) {
            if (isFirstScroll.value) {
                listState.scrollToItem(messageItems.size - 1)
                isFirstScroll.value = false
            } else {
                listState.animateScrollToItem(messageItems.size - 1)
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(FoundationColors.Background)
    ) {
        // ── Conversation header ───────────────────────────────────────────────
        ConversationHeader(
            partnerId    = partnerId,
            partnerColor = partnerColor,
            partner      = partner,
            onBack       = onBack
        )

        HorizontalDivider(color = FoundationColors.Surface, thickness = 1.dp)

        // ── Message area ──────────────────────────────────────────────────────
        Box(modifier = Modifier.weight(1f)) {
            if (messageItems.isEmpty()) {
                EmptyConversation(partnerId = partnerId, partnerColor = partnerColor)
            } else {
                LazyColumn(
                    state           = listState,
                    modifier        = Modifier.fillMaxSize(),
                    contentPadding  = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp)
                ) {
                    items(messageItems, key = { it.dm.id }) { item ->
                        if (item.showDateSeparator) {
                            DateSeparator(label = item.dateLabel)
                        }
                        MessageBubble(item = item, partnerColor = partnerColor)
                    }
                }
            }
        }

        HorizontalDivider(color = FoundationColors.Surface, thickness = 1.dp)

        // ── Input bar ─────────────────────────────────────────────────────────
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(FoundationColors.Surface)
                .padding(horizontal = 8.dp, vertical = 6.dp),
            verticalAlignment    = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            OutlinedTextField(
                value         = inputText,
                onValueChange = { inputText = it },
                placeholder   = {
                    Text(
                        text  = "Message $partnerId…",
                        fontSize = 13.sp,
                        color = FoundationColors.OnSurfaceVariant
                    )
                },
                modifier  = Modifier.weight(1f),
                maxLines  = 5,
                colors    = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor   = partnerColor.copy(alpha = 0.50f),
                    unfocusedBorderColor = FoundationColors.OnSurfaceVariant.copy(alpha = 0.30f),
                    cursorColor          = partnerColor,
                    focusedTextColor     = FoundationColors.OnSurface,
                    unfocusedTextColor   = FoundationColors.OnSurface
                )
            )

            val canSend = inputText.isNotBlank()
            IconButton(
                onClick  = {
                    val text = inputText.trim()
                    if (text.isNotEmpty()) {
                        onSendDm(partnerId, text)
                        inputText = ""
                    }
                },
                modifier = Modifier
                    .size(48.dp)
                    .clip(CircleShape)
                    .background(if (canSend) FoundationColors.Primary else FoundationColors.SurfaceVariant)
            ) {
                Icon(
                    imageVector        = Icons.AutoMirrored.Filled.Send,
                    contentDescription = "Send",
                    tint               = if (canSend) FoundationColors.Background else FoundationColors.OnSurfaceVariant
                )
            }
        }
    }
}

// ── Header ────────────────────────────────────────────────────────────────────

@Composable
private fun ConversationHeader(
    partnerId: String,
    partnerColor: Color,
    partner: TeamMember?,
    onBack: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(FoundationColors.Surface)
            .padding(horizontal = 4.dp, vertical = 8.dp),
        verticalAlignment     = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        IconButton(onClick = onBack) {
            Icon(
                imageVector        = Icons.AutoMirrored.Filled.ArrowBack,
                contentDescription = "Back",
                tint               = FoundationColors.OnSurface
            )
        }

        // Colored avatar circle with identity initial
        Box(
            modifier = Modifier
                .size(40.dp)
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

        // Name + presence subtitle
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text       = partnerId,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize   = 15.sp,
                color      = FoundationColors.OnSurface
            )
            val statusText = when {
                partner == null                              -> "not in team"
                partner.online && partner.activity != null  -> partner.activity!!
                partner.online                              -> "online"
                partner.last_seen.isNotBlank()              -> "offline · ${partner.last_seen}"
                else                                        -> "offline"
            }
            val statusColor = if (partner?.online == true) FoundationColors.Online else FoundationColors.OnSurfaceVariant
            Text(
                text     = statusText,
                style    = MaterialTheme.typography.bodySmall,
                color    = statusColor,
                maxLines = 1
            )
        }

        // Live presence dot (only when online)
        if (partner?.online == true) {
            Box(
                modifier = Modifier
                    .padding(end = 8.dp)
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(FoundationColors.Online)
            )
        }
    }
}

// ── Empty state ───────────────────────────────────────────────────────────────

@Composable
private fun EmptyConversation(partnerId: String, partnerColor: Color) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Box(
                modifier = Modifier
                    .size(72.dp)
                    .clip(CircleShape)
                    .background(AiIdentity.avatarBackground(partnerId))
                    .border(2.dp, AiIdentity.avatarBorder(partnerId), CircleShape),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text       = AiIdentity.initial(partnerId),
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    fontSize   = 28.sp,
                    color      = partnerColor
                )
            }
            Text(
                text  = partnerId,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize   = 15.sp,
                color      = FoundationColors.OnSurface
            )
            Text(
                text  = "No messages yet",
                style = MaterialTheme.typography.bodySmall,
                color = FoundationColors.OnSurfaceVariant
            )
            Text(
                text  = "Start the conversation below",
                style = MaterialTheme.typography.labelSmall,
                color = FoundationColors.OnSurfaceVariant.copy(alpha = 0.55f),
                fontFamily = FontFamily.Monospace
            )
        }
    }
}

// ── Date separator ────────────────────────────────────────────────────────────

@Composable
private fun DateSeparator(label: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 12.dp),
        horizontalArrangement = Arrangement.Center
    ) {
        Surface(
            shape = MaterialTheme.shapes.small,
            color = FoundationColors.SurfaceVariant
        ) {
            Text(
                text       = label,
                fontFamily = FontFamily.Monospace,
                fontSize   = 10.sp,
                color      = FoundationColors.OnSurfaceVariant,
                modifier   = Modifier.padding(horizontal = 10.dp, vertical = 3.dp)
            )
        }
    }
}

// ── Chat bubble ───────────────────────────────────────────────────────────────

@Composable
private fun MessageBubble(item: MessageItem, partnerColor: Color) {
    // Sent = right-aligned; received = left-aligned.
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(
                start  = if (item.isMine) 56.dp else 0.dp,
                end    = if (item.isMine) 0.dp   else 56.dp,
                top    = 2.dp,
                bottom = 2.dp
            ),
        horizontalAlignment = if (item.isMine) Alignment.End else Alignment.Start
    ) {
        if (item.isMine) {
            // ── Sent bubble — primary green, dark text ──────────────────────
            val sentShape = RoundedCornerShape(topStart = 18.dp, topEnd = 4.dp, bottomStart = 18.dp, bottomEnd = 18.dp)
            Box(
                modifier = Modifier
                    .clip(sentShape)
                    .background(FoundationColors.Primary)
                    .padding(horizontal = 14.dp, vertical = 10.dp)
            ) {
                Text(
                    text       = item.dm.content,
                    color      = FoundationColors.Background,   // near-black on green — 7.4:1 contrast, WCAG AA ✓
                    fontSize   = 14.sp,
                    lineHeight = 20.sp
                )
            }
        } else {
            // ── Received bubble — partner-color tint + border ───────────────
            val receivedShape = RoundedCornerShape(topStart = 4.dp, topEnd = 18.dp, bottomStart = 18.dp, bottomEnd = 18.dp)
            Box(
                modifier = Modifier
                    .border(1.dp, partnerColor.copy(alpha = 0.28f), receivedShape)
                    .clip(receivedShape)
                    .background(partnerColor.copy(alpha = 0.10f))
                    .padding(horizontal = 14.dp, vertical = 10.dp)
            ) {
                Text(
                    text       = item.dm.content,
                    color      = FoundationColors.OnSurface,
                    fontSize   = 14.sp,
                    lineHeight = 20.sp
                )
            }
        }

        // Timestamp below each bubble
        Text(
            text       = item.formattedTime,
            style      = MaterialTheme.typography.labelSmall,
            color      = FoundationColors.OnSurfaceVariant.copy(alpha = 0.55f),
            fontFamily = FontFamily.Monospace,
            modifier   = Modifier.padding(horizontal = 4.dp, top = 2.dp)
        )
    }
}

// ── Data model + helpers ──────────────────────────────────────────────────────

private data class MessageItem(
    val dm: Dm,
    val isMine: Boolean,
    val showDateSeparator: Boolean,
    val dateLabel: String,
    val formattedTime: String
)

private fun buildMessageItems(dms: List<Dm>, myHId: String): List<MessageItem> =
    dms.mapIndexed { i, dm ->
        val dmDate   = parseLocalDate(dm.timestamp)
        val prevDate = if (i > 0) parseLocalDate(dms[i - 1].timestamp) else null
        MessageItem(
            dm                = dm,
            isMine            = dm.from == myHId,
            showDateSeparator = prevDate == null || dmDate != prevDate,
            dateLabel         = formatDateLabel(dmDate),
            formattedTime     = formatMessageTime(dm.timestamp)
        )
    }

private fun parseLocalDate(timestamp: String): LocalDate = try {
    Instant.parse(timestamp).atZone(ZoneId.systemDefault()).toLocalDate()
} catch (_: Exception) {
    LocalDate.now()
}

private fun formatDateLabel(date: LocalDate): String {
    val today = LocalDate.now()
    return when (date) {
        today              -> "TODAY"
        today.minusDays(1) -> "YESTERDAY"
        else               -> date.format(DateTimeFormatter.ofPattern("MMM d, yyyy")).uppercase()
    }
}

private fun formatMessageTime(timestamp: String): String = try {
    Instant.parse(timestamp)
        .atZone(ZoneId.systemDefault())
        .format(DateTimeFormatter.ofPattern("HH:mm"))
} catch (_: Exception) {
    timestamp.take(5)
}
