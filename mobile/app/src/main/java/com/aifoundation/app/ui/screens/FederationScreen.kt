package com.aifoundation.app.ui.screens

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Help
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.*
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors
import java.time.Instant

/**
 * Federation Screen - Shows the Deep Net network visualization
 * Displays connected nodes (AIs, humans, servers) and network status
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FederationScreen(
    connectionState: ConnectionState,
    wallStatus: WallStatus,
    nodes: List<FederationNode>,
    stats: FederationStats?,
    onRefresh: () -> Unit,
    onNodeClick: (FederationNode) -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // Header with Deep Net branding
        DeepNetHeader(connectionState, wallStatus)

        // Stats bar
        stats?.let { NetworkStatsBar(it) }

        // Node list
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            // Section: AI Agents
            val aiNodes = nodes.filter { it.entityType == EntityType.AI_AGENT }
            if (aiNodes.isNotEmpty()) {
                item {
                    SectionHeader(
                        title = "AI TEAM",
                        subtitle = "${aiNodes.size} agent${if (aiNodes.size != 1) "s" else ""} connected",
                        icon = Icons.Default.Memory,
                        color = DeepNetColors.Secondary
                    )
                }
                items(aiNodes) { node ->
                    NodeCard(node = node, onClick = { onNodeClick(node) })
                }
            }

            // Section: Human Users
            val humanNodes = nodes.filter {
                it.entityType == EntityType.HUMAN_MOBILE || it.entityType == EntityType.HUMAN_DESKTOP
            }
            if (humanNodes.isNotEmpty()) {
                item {
                    Spacer(modifier = Modifier.height(16.dp))
                    SectionHeader(
                        title = "USERS",
                        subtitle = "${humanNodes.size} device${if (humanNodes.size != 1) "s" else ""} connected",
                        icon = Icons.Default.Person,
                        color = DeepNetColors.Primary
                    )
                }
                items(humanNodes) { node ->
                    NodeCard(node = node, onClick = { onNodeClick(node) })
                }
            }

            // Section: Infrastructure
            val serverNodes = nodes.filter { it.entityType == EntityType.SERVER }
            if (serverNodes.isNotEmpty()) {
                item {
                    Spacer(modifier = Modifier.height(16.dp))
                    SectionHeader(
                        title = "INFRASTRUCTURE",
                        subtitle = "Servers & Gateways",
                        icon = Icons.Default.Dns,
                        color = DeepNetColors.OnSurfaceVariant
                    )
                }
                items(serverNodes) { node ->
                    NodeCard(node = node, onClick = { onNodeClick(node) })
                }
            }
        }
    }
}

@Composable
fun DeepNetHeader(connectionState: ConnectionState, wallStatus: WallStatus) {
    val infiniteTransition = rememberInfiniteTransition(label = "pulse")
    val pulseAlpha by infiniteTransition.animateFloat(
        initialValue = 0.3f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(1000),
            repeatMode = RepeatMode.Reverse
        ),
        label = "pulse"
    )

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
                bracketLength = 20.dp,
                strokeWidth = 2.dp,
                animated = true
            )
            .padding(16.dp)
    ) {
        Column {
            // Title row
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
                modifier = Modifier.fillMaxWidth()
            ) {
                Column {
                    Text(
                        text = "TEAMBOOK",
                        fontSize = 28.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = FontFamily.Monospace,
                        color = DeepNetColors.Primary
                    )
                    Text(
                        text = "Network Status",
                        fontSize = 12.sp,
                        color = DeepNetColors.OnSurfaceVariant
                    )
                }

                // Connection indicator
                ConnectionIndicator(connectionState, pulseAlpha)
            }

        }
    }
}

@Composable
fun ConnectionIndicator(state: ConnectionState, pulseAlpha: Float) {
    val color = when (state) {
        ConnectionState.CONNECTED, ConnectionState.AUTHENTICATED -> DeepNetColors.Online
        ConnectionState.CONNECTING -> DeepNetColors.Warning
        ConnectionState.DISCONNECTED -> DeepNetColors.Offline
        ConnectionState.ERROR -> DeepNetColors.Error
    }

    val text = when (state) {
        ConnectionState.CONNECTED -> "Connected"
        ConnectionState.AUTHENTICATED -> "Connected"
        ConnectionState.CONNECTING -> "Connecting..."
        ConnectionState.DISCONNECTED -> "Offline"
        ConnectionState.ERROR -> "Error"
    }

    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Box(
            modifier = Modifier
                .size(12.dp)
                .clip(CircleShape)
                .background(color.copy(alpha = if (state == ConnectionState.CONNECTING) pulseAlpha else 1f))
        )
        Text(
            text = text,
            fontSize = 12.sp,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            color = color
        )
    }
}

@Composable
fun NetworkStatsBar(stats: FederationStats) {
    val shape = DeepNetShapes.DataStream

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp)
            .clip(shape)
            .background(DeepNetColors.Surface, shape)
            .border(1.dp, DeepNetColors.Primary.copy(alpha = 0.4f), shape)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.SpaceEvenly
    ) {
        StatItem(label = "Total", value = stats.totalNodes.toString())
        StatItem(label = "AIs", value = stats.aiAgents.toString())
        StatItem(label = "Users", value = stats.humanUsers.toString())
        StatItem(label = "Messages", value = stats.messagesLast24h.toString())
    }
}

@Composable
fun StatItem(label: String, value: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            text = value,
            fontSize = 18.sp,
            fontWeight = FontWeight.Bold,
            fontFamily = FontFamily.Monospace,
            color = DeepNetColors.Primary
        )
        Text(
            text = label,
            fontSize = 10.sp,
            color = DeepNetColors.OnSurfaceVariant
        )
    }
}

@Composable
fun SectionHeader(title: String, subtitle: String, icon: androidx.compose.ui.graphics.vector.ImageVector, color: Color) {
    DeepNetSectionHeader(
        title = title,
        subtitle = subtitle,
        icon = icon,
        accentColor = color,
        modifier = Modifier.padding(vertical = 8.dp)
    )
}

@Composable
fun NodeCard(node: FederationNode, onClick: () -> Unit) {
    val statusColor = when (node.status) {
        NodeStatus.ONLINE -> DeepNetColors.Online
        NodeStatus.AWAY -> DeepNetColors.Warning
        NodeStatus.BUSY -> DeepNetColors.Secondary
        NodeStatus.OFFLINE -> DeepNetColors.Offline
    }

    val typeIcon = when (node.entityType) {
        EntityType.AI_AGENT -> Icons.Default.Memory
        EntityType.HUMAN_MOBILE -> Icons.Default.PhoneAndroid
        EntityType.HUMAN_DESKTOP -> Icons.Default.Computer
        EntityType.SERVER -> Icons.Default.Dns
        EntityType.UNKNOWN -> Icons.AutoMirrored.Filled.Help
    }

    val cardVariant = when (node.entityType) {
        EntityType.AI_AGENT -> DeepNetCardVariant.NODE
        EntityType.SERVER -> DeepNetCardVariant.TERMINAL
        else -> DeepNetCardVariant.STANDARD
    }

    DeepNetCard(
        modifier = Modifier.fillMaxWidth(),
        variant = cardVariant,
        onClick = onClick,
        enableGlow = node.status == NodeStatus.ONLINE && node.entityType == EntityType.AI_AGENT
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Status indicator
            DeepNetStatusIndicator(
                status = when (node.status) {
                    NodeStatus.ONLINE -> DeepNetStatus.ONLINE
                    NodeStatus.AWAY -> DeepNetStatus.CONNECTING
                    NodeStatus.BUSY -> DeepNetStatus.CONNECTING
                    NodeStatus.OFFLINE -> DeepNetStatus.OFFLINE
                },
                size = 10.dp,
                animated = node.status != NodeStatus.OFFLINE
            )

            Spacer(modifier = Modifier.width(12.dp))

            // Type icon
            Icon(
                imageVector = typeIcon,
                contentDescription = null,
                tint = DeepNetColors.OnSurfaceVariant,
                modifier = Modifier.size(24.dp)
            )

            Spacer(modifier = Modifier.width(12.dp))

            // Node info
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = node.displayName,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.Medium,
                    color = DeepNetColors.OnSurface
                )
                Text(
                    text = node.id,
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.OnSurfaceVariant
                )
                node.currentActivity?.let {
                    Text(
                        text = it,
                        fontSize = 10.sp,
                        color = DeepNetColors.Primary.copy(alpha = 0.7f)
                    )
                }
            }

            // Chevron
            Icon(
                imageVector = Icons.Default.ChevronRight,
                contentDescription = null,
                tint = DeepNetColors.OnSurfaceVariant
            )
        }
    }
}
