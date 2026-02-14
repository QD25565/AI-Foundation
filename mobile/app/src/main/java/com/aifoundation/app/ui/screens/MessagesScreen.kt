package com.aifoundation.app.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.*
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.ripple
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.*
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * Messages Screen - Social-style feed showing Broadcasts and DMs from the Deep Net
 * Decentralized messaging without central backend dependency
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MessagesScreen(
    messages: List<DeepNetMessage>,
    currentUserId: String,
    onSendMessage: (String, String?) -> Unit, // content, recipientId (null = broadcast)
    onRefresh: () -> Unit
) {
    var selectedFeed by remember { mutableStateOf(FeedType.ALL) }
    var messageText by remember { mutableStateOf("") }
    var isBroadcast by remember { mutableStateOf(true) }
    var recipient by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // Styled header
        MessagesHeader()

        // Feed type selector
        FeedTypeSelector(
            selectedFeed = selectedFeed,
            onFeedSelected = { selectedFeed = it },
            broadcastCount = messages.count { it.messageType == MessageType.BROADCAST },
            dmCount = messages.count { it.messageType == MessageType.DIRECT }
        )

        // Messages feed
        val filteredMessages = when (selectedFeed) {
            FeedType.ALL -> messages
            FeedType.BROADCASTS -> messages.filter { it.messageType == MessageType.BROADCAST }
            FeedType.DIRECT -> messages.filter { it.messageType == MessageType.DIRECT }
        }

        if (filteredMessages.isEmpty()) {
            Box(modifier = Modifier.weight(1f)) {
                EmptyMessagesState(feedType = selectedFeed)
            }
        } else {
            SocialMessageFeed(
                messages = filteredMessages,
                currentUserId = currentUserId,
                modifier = Modifier.weight(1f)
            )
        }

        // Message compose area at bottom
        MessageComposeArea(
            messageText = messageText,
            onMessageTextChange = { messageText = it },
            isBroadcast = isBroadcast,
            onBroadcastChange = { isBroadcast = it },
            recipient = recipient,
            onRecipientChange = { recipient = it },
            onSend = {
                if (messageText.isNotBlank()) {
                    onSendMessage(messageText, if (isBroadcast) null else recipient.ifBlank { null })
                    messageText = ""
                }
            }
        )
    }
}

enum class FeedType { ALL, BROADCASTS, DIRECT }

/**
 * Message compose area - fixed at bottom of screen
 */
@Composable
fun MessageComposeArea(
    messageText: String,
    onMessageTextChange: (String) -> Unit,
    isBroadcast: Boolean,
    onBroadcastChange: (Boolean) -> Unit,
    recipient: String,
    onRecipientChange: (String) -> Unit,
    onSend: () -> Unit
) {
    val shape = DeepNetShapes.Terminal

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(DeepNetColors.Surface, shape)
            .border(1.dp, DeepNetColors.Primary.copy(alpha = 0.5f), shape)
            .padding(12.dp)
    ) {
        // Type selector row
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Broadcast / DM toggle
            Row(
                modifier = Modifier
                    .clip(DeepNetShapes.SmallCut)
                    .background(DeepNetColors.Background)
                    .padding(2.dp)
            ) {
                Box(
                    modifier = Modifier
                        .clip(DeepNetShapes.SmallCut)
                        .background(
                            if (isBroadcast) DeepNetColors.Secondary.copy(alpha = 0.3f)
                            else DeepNetColors.Background
                        )
                        .clickable { onBroadcastChange(true) }
                        .padding(horizontal = 12.dp, vertical = 6.dp)
                ) {
                    Text(
                        text = "Broadcast",
                        fontSize = 12.sp,
                        fontFamily = FontFamily.Monospace,
                        color = if (isBroadcast) DeepNetColors.Secondary else DeepNetColors.OnSurfaceVariant
                    )
                }
                Box(
                    modifier = Modifier
                        .clip(DeepNetShapes.SmallCut)
                        .background(
                            if (!isBroadcast) DeepNetColors.Primary.copy(alpha = 0.3f)
                            else DeepNetColors.Background
                        )
                        .clickable { onBroadcastChange(false) }
                        .padding(horizontal = 12.dp, vertical = 6.dp)
                ) {
                    Text(
                        text = "Direct",
                        fontSize = 12.sp,
                        fontFamily = FontFamily.Monospace,
                        color = if (!isBroadcast) DeepNetColors.Primary else DeepNetColors.OnSurfaceVariant
                    )
                }
            }

            // Recipient field (only for DM)
            if (!isBroadcast) {
                OutlinedTextField(
                    value = recipient,
                    onValueChange = onRecipientChange,
                    placeholder = { Text("Recipient", fontSize = 12.sp) },
                    modifier = Modifier
                        .weight(1f)
                        .height(40.dp),
                    textStyle = LocalTextStyle.current.copy(fontSize = 12.sp),
                    singleLine = true,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = DeepNetColors.Primary,
                        unfocusedBorderColor = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.5f),
                        cursorColor = DeepNetColors.Primary
                    )
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Message input row
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.Bottom
        ) {
            OutlinedTextField(
                value = messageText,
                onValueChange = onMessageTextChange,
                placeholder = {
                    Text(
                        if (isBroadcast) "Write a broadcast..." else "Write a message...",
                        fontSize = 14.sp
                    )
                },
                modifier = Modifier
                    .weight(1f)
                    .heightIn(min = 48.dp, max = 120.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = if (isBroadcast) DeepNetColors.Secondary else DeepNetColors.Primary,
                    unfocusedBorderColor = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.5f),
                    cursorColor = if (isBroadcast) DeepNetColors.Secondary else DeepNetColors.Primary
                )
            )

            // Send button
            val canSend = messageText.isNotBlank() && (isBroadcast || recipient.isNotBlank())
            IconButton(
                onClick = onSend,
                enabled = canSend,
                modifier = Modifier
                    .size(48.dp)
                    .clip(DeepNetShapes.SmallCut)
                    .background(
                        if (canSend) {
                            if (isBroadcast) DeepNetColors.Secondary else DeepNetColors.Primary
                        } else {
                            DeepNetColors.OnSurfaceVariant.copy(alpha = 0.3f)
                        }
                    )
            ) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.Send,
                    contentDescription = "Send",
                    tint = if (canSend) DeepNetColors.OnPrimary else DeepNetColors.OnSurfaceVariant
                )
            }
        }
    }
}

@Composable
fun MessagesHeader() {
    val headerShape = DeepNetShapes.Header

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 8.dp)
            .clip(headerShape)
            .background(
                Brush.verticalGradient(
                    colors = listOf(
                        DeepNetColors.SurfaceVariant,
                        DeepNetColors.Surface
                    )
                ),
                headerShape
            )
            .border(1.dp, DeepNetColors.Primary.copy(alpha = 0.5f), headerShape)
            .deepNetCornerBrackets(
                enabled = true,
                bracketColor = DeepNetColors.Primary,
                bracketLength = 16.dp,
                strokeWidth = 2.dp,
                animated = false
            )
            .padding(16.dp)
    ) {
        Column {
            Text(
                text = "MESSAGES",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                color = DeepNetColors.Primary
            )
            Text(
                text = "Broadcasts & Direct Messages",
                fontSize = 12.sp,
                color = DeepNetColors.OnSurfaceVariant
            )
        }
    }
}

@Composable
fun FeedTypeSelector(
    selectedFeed: FeedType,
    onFeedSelected: (FeedType) -> Unit,
    broadcastCount: Int,
    dmCount: Int
) {
    val shape = DeepNetShapes.DataStream

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp)
            .clip(shape)
            .background(DeepNetColors.Surface, shape)
            .border(1.dp, DeepNetColors.Primary.copy(alpha = 0.4f), shape)
            .padding(4.dp),
        horizontalArrangement = Arrangement.SpaceEvenly
    ) {
        FeedTab(
            icon = Icons.Default.Stream,
            label = "ALL",
            selected = selectedFeed == FeedType.ALL,
            onClick = { onFeedSelected(FeedType.ALL) },
            modifier = Modifier.weight(1f)
        )
        FeedTab(
            icon = Icons.Default.Campaign,
            label = "BROADCASTS",
            count = broadcastCount,
            selected = selectedFeed == FeedType.BROADCASTS,
            onClick = { onFeedSelected(FeedType.BROADCASTS) },
            modifier = Modifier.weight(1f)
        )
        FeedTab(
            icon = Icons.Default.Person,
            label = "DIRECT",
            count = dmCount,
            selected = selectedFeed == FeedType.DIRECT,
            onClick = { onFeedSelected(FeedType.DIRECT) },
            modifier = Modifier.weight(1f)
        )
    }
}

@Composable
fun FeedTab(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    count: Int? = null
) {
    val tabShape = DeepNetShapes.SmallCut

    Box(
        modifier = modifier
            .clip(tabShape)
            .background(
                if (selected) DeepNetColors.Primary.copy(alpha = 0.2f) else DeepNetColors.Surface,
                tabShape
            )
            .clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = ripple(color = DeepNetColors.Primary),
                onClick = onClick
            )
            .padding(vertical = 10.dp, horizontal = 8.dp),
        contentAlignment = Alignment.Center
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = if (selected) DeepNetColors.Primary else DeepNetColors.OnSurfaceVariant,
                modifier = Modifier.size(18.dp)
            )
            Spacer(modifier = Modifier.width(6.dp))
            Text(
                text = label,
                fontSize = 11.sp,
                fontFamily = FontFamily.Monospace,
                fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal,
                color = if (selected) DeepNetColors.Primary else DeepNetColors.OnSurfaceVariant
            )
            count?.let {
                if (it > 0) {
                    Spacer(modifier = Modifier.width(4.dp))
                    Box(
                        modifier = Modifier
                            .size(16.dp)
                            .clip(CircleShape)
                            .background(DeepNetColors.Primary.copy(alpha = 0.3f)),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            text = if (it > 99) "99+" else it.toString(),
                            fontSize = 8.sp,
                            color = DeepNetColors.Primary
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun SocialMessageFeed(
    messages: List<DeepNetMessage>,
    currentUserId: String,
    modifier: Modifier = Modifier
) {
    // Group messages by date
    val groupedMessages = messages.groupBy { msg ->
        msg.timestamp.atZone(ZoneId.systemDefault()).toLocalDate()
    }.toSortedMap(reverseOrder())

    LazyColumn(
        modifier = modifier.fillMaxWidth(),
        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        groupedMessages.forEach { (date, dateMessages) ->
            // Date header
            item {
                DateHeader(date = date)
            }

            // Messages for this date
            items(dateMessages.sortedByDescending { it.timestamp }) { message ->
                SocialMessageCard(
                    message = message,
                    isOwnMessage = message.from == currentUserId
                )
            }
        }
    }
}

@Composable
fun DateHeader(date: LocalDate) {
    val today = LocalDate.now()
    val yesterday = today.minusDays(1)

    val displayText = when (date) {
        today -> "TODAY"
        yesterday -> "YESTERDAY"
        else -> date.format(DateTimeFormatter.ofPattern("MMM d, yyyy")).uppercase()
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        DeepNetDivider(modifier = Modifier.weight(1f))
        Text(
            text = displayText,
            fontSize = 10.sp,
            fontFamily = FontFamily.Monospace,
            color = DeepNetColors.OnSurfaceVariant,
            modifier = Modifier.padding(horizontal = 12.dp)
        )
        DeepNetDivider(modifier = Modifier.weight(1f))
    }
}

/**
 * Social-style message card with enhanced visuals
 */
@Composable
fun SocialMessageCard(message: DeepNetMessage, isOwnMessage: Boolean) {
    val typeColor = when (message.messageType) {
        MessageType.DIRECT -> DeepNetColors.Primary
        MessageType.BROADCAST -> DeepNetColors.Secondary
        MessageType.SYSTEM -> DeepNetColors.OnSurfaceVariant
        MessageType.ALERT -> DeepNetColors.Error
    }

    val typeIcon = when (message.messageType) {
        MessageType.DIRECT -> Icons.Default.Person
        MessageType.BROADCAST -> Icons.Default.Campaign
        MessageType.SYSTEM -> Icons.Default.Info
        MessageType.ALERT -> Icons.Default.Warning
    }

    val cardVariant = when (message.messageType) {
        MessageType.DIRECT -> DeepNetCardVariant.DATA
        MessageType.BROADCAST -> DeepNetCardVariant.FEDERATION
        MessageType.SYSTEM -> DeepNetCardVariant.TERMINAL
        MessageType.ALERT -> DeepNetCardVariant.ALERT
    }

    DeepNetCard(
        modifier = Modifier.fillMaxWidth(),
        variant = cardVariant,
        enablePulse = message.messageType == MessageType.ALERT && !message.isRead,
        enableGlow = message.messageType == MessageType.BROADCAST && !message.isRead
    ) {
        // Type badge at top
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Message type badge
            Row(
                modifier = Modifier
                    .clip(DeepNetShapes.SmallCut)
                    .background(typeColor.copy(alpha = 0.15f))
                    .padding(horizontal = 8.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                Icon(
                    imageVector = typeIcon,
                    contentDescription = null,
                    tint = typeColor,
                    modifier = Modifier.size(12.dp)
                )
                Text(
                    text = when (message.messageType) {
                        MessageType.DIRECT -> "DM"
                        MessageType.BROADCAST -> "BROADCAST"
                        MessageType.SYSTEM -> "SYSTEM"
                        MessageType.ALERT -> "ALERT"
                    },
                    fontSize = 9.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color = typeColor
                )
            }

            // Timestamp
            Text(
                text = formatTimestamp(message.timestamp),
                fontSize = 10.sp,
                fontFamily = FontFamily.Monospace,
                color = DeepNetColors.OnSurfaceVariant
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Sender info
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            // Avatar circle
            Box(
                modifier = Modifier
                    .size(32.dp)
                    .clip(CircleShape)
                    .background(
                        if (isOwnMessage) DeepNetColors.Primary.copy(alpha = 0.2f)
                        else DeepNetColors.Secondary.copy(alpha = 0.2f)
                    )
                    .border(
                        1.dp,
                        if (isOwnMessage) DeepNetColors.Primary else DeepNetColors.Secondary,
                        CircleShape
                    ),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = message.from.firstOrNull()?.uppercase() ?: "?",
                    fontSize = 14.sp,
                    fontWeight = FontWeight.Bold,
                    fontFamily = FontFamily.Monospace,
                    color = if (isOwnMessage) DeepNetColors.Primary else DeepNetColors.Secondary
                )
            }

            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = message.from,
                        fontSize = 13.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = FontFamily.Monospace,
                        color = if (isOwnMessage) DeepNetColors.Primary else DeepNetColors.OnSurface
                    )
                    if (isOwnMessage) {
                        Spacer(modifier = Modifier.width(4.dp))
                        Text(
                            text = "(you)",
                            fontSize = 10.sp,
                            color = DeepNetColors.OnSurfaceVariant
                        )
                    }
                }

                // Recipient for DMs
                if (message.messageType == MessageType.DIRECT && message.to != null) {
                    Text(
                        text = "→ ${message.to}",
                        fontSize = 11.sp,
                        fontFamily = FontFamily.Monospace,
                        color = DeepNetColors.OnSurfaceVariant
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(10.dp))

        // Message content
        Text(
            text = message.content,
            fontSize = 14.sp,
            color = DeepNetColors.OnSurface,
            lineHeight = 20.sp
        )

        // Unread indicator
        if (!message.isRead && !isOwnMessage) {
            Spacer(modifier = Modifier.height(8.dp))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                DeepNetStatusIndicator(
                    status = DeepNetStatus.ONLINE,
                    size = 8.dp,
                    animated = true
                )
                Text(
                    text = "NEW",
                    fontSize = 9.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color = DeepNetColors.Online
                )
            }
        }
    }
}

@Composable
fun EmptyMessagesState(feedType: FeedType) {
    val (icon, title, subtitle) = when (feedType) {
        FeedType.ALL -> Triple(
            Icons.Default.Forum,
            "No messages yet",
            "Start a conversation or send a broadcast"
        )
        FeedType.BROADCASTS -> Triple(
            Icons.Default.Campaign,
            "No broadcasts",
            "Send a broadcast to share with everyone"
        )
        FeedType.DIRECT -> Triple(
            Icons.Default.Person,
            "No direct messages",
            "Send a DM to connect privately"
        )
    }

    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        DeepNetCard(
            modifier = Modifier
                .fillMaxWidth(0.8f)
                .padding(16.dp),
            variant = DeepNetCardVariant.TERMINAL
        ) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(24.dp)
            ) {
                Box(
                    modifier = Modifier
                        .size(72.dp)
                        .deepNetCornerBrackets(
                            enabled = true,
                            bracketColor = DeepNetColors.OnSurfaceVariant,
                            bracketLength = 12.dp,
                            strokeWidth = 1.dp,
                            animated = false
                        ),
                    contentAlignment = Alignment.Center
                ) {
                    Icon(
                        imageVector = icon,
                        contentDescription = null,
                        tint = DeepNetColors.OnSurfaceVariant,
                        modifier = Modifier.size(40.dp)
                    )
                }
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    text = title.uppercase(),
                    fontSize = 14.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    color = DeepNetColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = subtitle,
                    fontSize = 12.sp,
                    color = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.7f),
                    textAlign = TextAlign.Center
                )
            }
        }
    }
}

private fun formatTimestamp(instant: Instant): String {
    val now = Instant.now()
    val duration = Duration.between(instant, now)

    return when {
        duration.toMinutes() < 1 -> "now"
        duration.toMinutes() < 60 -> "${duration.toMinutes()}m ago"
        duration.toHours() < 24 -> "${duration.toHours()}h ago"
        else -> {
            val formatter = DateTimeFormatter.ofPattern("MMM d")
                .withZone(ZoneId.systemDefault())
            formatter.format(instant)
        }
    }
}
