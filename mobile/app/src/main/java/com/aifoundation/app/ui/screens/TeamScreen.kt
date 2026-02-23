package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.data.model.TeamMember
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors

/**
 * Team roster — AI agents + humans with real-time presence.
 * Updated via SSE team_updated events.
 */
@Composable
fun TeamScreen(
    team: List<TeamMember>,
    onRefresh: () -> Unit,
    onSendDm: (String) -> Unit,   // recipient ai_id → navigate to Inbox
    isLoading: Boolean
) {
    val aiMembers    = team.filter { it.isAi }
    val humanMembers = team.filter { !it.isAi }
    val onlineCount  = team.count { it.online }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
    ) {
        // Header
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column {
                Text(
                    text       = "TEAM",
                    style      = MaterialTheme.typography.headlineSmall,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Black,
                    color      = DeepNetColors.Primary
                )
                if (team.isNotEmpty()) {
                    Text(
                        text       = "$onlineCount of ${team.size} online",
                        style      = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color      = DeepNetColors.OnSurfaceVariant
                    )
                }
            }
            DeepNetButton(
                onClick = onRefresh,
                variant = DeepNetButtonVariant.GHOST,
                icon    = Icons.Default.Refresh,
                text    = "REFRESH"
            )
        }

        if (isLoading) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetLoadingIndicator(text = "LOADING TEAM...")
            }
            return@Column
        }

        if (team.isEmpty()) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                DeepNetCard(modifier = Modifier.fillMaxWidth(0.8f), variant = DeepNetCardVariant.TERMINAL) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.fillMaxWidth().padding(32.dp)
                    ) {
                        Icon(
                            imageVector = Icons.Default.Groups,
                            contentDescription = null,
                            tint = DeepNetColors.OnSurfaceVariant,
                            modifier = Modifier.size(48.dp)
                        )
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text       = "NO TEAM MEMBERS",
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
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 4.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp)
        ) {
            // AI agents section
            if (aiMembers.isNotEmpty()) {
                item {
                    Text(
                        text       = "AI AGENTS",
                        style      = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color      = DeepNetColors.OnSurfaceVariant,
                        modifier   = Modifier.padding(horizontal = 4.dp, vertical = 6.dp)
                    )
                }
                items(aiMembers, key = { it.ai_id }) { member ->
                    TeamMemberRow(member = member, onSendDm = onSendDm)
                }
            }

            // Humans section
            if (humanMembers.isNotEmpty()) {
                item {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text       = "HUMANS",
                        style      = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color      = DeepNetColors.OnSurfaceVariant,
                        modifier   = Modifier.padding(horizontal = 4.dp, vertical = 6.dp)
                    )
                }
                items(humanMembers, key = { it.ai_id }) { member ->
                    TeamMemberRow(member = member, onSendDm = onSendDm)
                }
            }

            item { Spacer(modifier = Modifier.height(8.dp)) }
        }
    }
}

@Composable
private fun TeamMemberRow(
    member: TeamMember,
    onSendDm: (String) -> Unit
) {
    DeepNetCard(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { if (member.isAi) onSendDm(member.ai_id) },
        variant  = DeepNetCardVariant.DATA
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            // Presence dot
            Box(
                modifier = Modifier
                    .size(10.dp)
                    .clip(CircleShape)
                    .background(if (member.online) DeepNetColors.Online else DeepNetColors.Offline)
            )

            // Avatar circle with initial
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .clip(CircleShape)
                    .background(DeepNetColors.SurfaceVariant),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text       = member.ai_id.take(1).uppercase(),
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    fontSize   = 14.sp,
                    color      = DeepNetColors.Primary
                )
            }

            // Name + activity
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text       = member.ai_id,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                    fontSize   = 14.sp,
                    color      = DeepNetColors.OnSurface
                )
                val subtitle = when {
                    !member.online         -> "offline · ${member.last_seen}"
                    member.activity != null -> member.activity
                    else                   -> "online"
                }
                Text(
                    text   = subtitle,
                    style  = MaterialTheme.typography.bodySmall,
                    color  = if (member.online) DeepNetColors.OnSurfaceVariant else DeepNetColors.Offline,
                    maxLines = 1
                )
            }

            // Type badge
            val (badgeLabel, badgeColor) = when (member.type) {
                "ai"    -> "AI"    to DeepNetColors.Primary
                "human" -> "HUMAN" to DeepNetColors.Warning
                else    -> member.type.uppercase() to DeepNetColors.Secondary
            }
            Surface(
                shape = MaterialTheme.shapes.small,
                color = badgeColor.copy(alpha = 0.15f)
            ) {
                Text(
                    text       = badgeLabel,
                    fontFamily = FontFamily.Monospace,
                    fontSize   = 9.sp,
                    fontWeight = FontWeight.Bold,
                    color      = badgeColor,
                    modifier   = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
                )
            }

            // DM arrow for AI members
            if (member.isAi) {
                Icon(
                    imageVector = Icons.Default.ChevronRight,
                    contentDescription = "Send DM",
                    tint = DeepNetColors.OnSurfaceVariant,
                    modifier = Modifier.size(16.dp)
                )
            }
        }
    }
}
