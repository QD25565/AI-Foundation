package com.aifoundation.app.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.material3.ripple
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.ui.theme.FoundationColors

/**
 * Themed UI components
 * Network-inspired visual design for the AI-Foundation mobile app
 */

/**
 * Card variant types for different contexts
 */
enum class FoundationCardVariant {
    STANDARD,       // Default card style
    NODE,           // For network node display
    TERMINAL,       // Terminal/console style
    DATA,           // Data display cards
    FEDERATION,     // Federation-related content
    ALERT,          // Alerts and warnings
    SUCCESS         // Success/confirmation
}

/**
 * Styled card with cut corners and optional effects
 */
@Composable
fun FoundationCard(
    modifier: Modifier = Modifier,
    variant: FoundationCardVariant = FoundationCardVariant.STANDARD,
    onClick: (() -> Unit)? = null,
    enablePulse: Boolean = false,
    enableGlow: Boolean = false,
    enableBrackets: Boolean = false,
    content: @Composable ColumnScope.() -> Unit
) {
    val shape = when (variant) {
        FoundationCardVariant.STANDARD -> FoundationShapes.Standard
        FoundationCardVariant.NODE -> FoundationShapes.Node
        FoundationCardVariant.TERMINAL -> FoundationShapes.Terminal
        FoundationCardVariant.DATA -> FoundationShapes.DataStream
        FoundationCardVariant.FEDERATION -> FoundationShapes.Federation
        FoundationCardVariant.ALERT -> FoundationShapes.Alert
        FoundationCardVariant.SUCCESS -> FoundationShapes.Standard
    }

    // All cards get visible asparagus green border by default
    val (backgroundColor, borderColor) = when (variant) {
        FoundationCardVariant.STANDARD -> FoundationColors.Surface to FoundationColors.Primary.copy(alpha = 0.6f)
        FoundationCardVariant.NODE -> FoundationColors.Surface to FoundationColors.Primary.copy(alpha = 0.7f)
        FoundationCardVariant.TERMINAL -> FoundationColors.Background to FoundationColors.Primary.copy(alpha = 0.8f)
        FoundationCardVariant.DATA -> FoundationColors.Surface to FoundationColors.Primary.copy(alpha = 0.5f)
        FoundationCardVariant.FEDERATION -> FoundationColors.Surface to FoundationColors.Primary.copy(alpha = 0.6f)
        FoundationCardVariant.ALERT -> FoundationColors.Surface to FoundationColors.Error.copy(alpha = 0.7f)
        FoundationCardVariant.SUCCESS -> FoundationColors.Surface to FoundationColors.Online.copy(alpha = 0.7f)
    }

    val pulseColor = when (variant) {
        FoundationCardVariant.ALERT -> FoundationColors.Error
        FoundationCardVariant.SUCCESS -> FoundationColors.Online
        else -> FoundationColors.Primary
    }

    var cardModifier = modifier
        .clip(shape)
        .background(backgroundColor, shape)
        .border(1.dp, borderColor, shape)

    if (enablePulse) {
        cardModifier = cardModifier.foundationEnergyPulse(
            enabled = true,
            pulseColor = pulseColor,
            shape = shape
        )
    }

    if (enableGlow) {
        cardModifier = cardModifier.foundationGlow(
            enabled = true,
            glowColor = pulseColor,
            shape = shape
        )
    }

    if (enableBrackets) {
        cardModifier = cardModifier.foundationCornerBrackets(
            enabled = true,
            bracketColor = borderColor
        )
    }

    if (onClick != null) {
        cardModifier = cardModifier.clickable(
            interactionSource = remember { MutableInteractionSource() },
            indication = ripple(color = FoundationColors.Primary),
            onClick = onClick
        )
    }

    Column(
        modifier = cardModifier.padding(12.dp),
        content = content
    )
}

/**
 * Styled button with cut corners
 */
@Composable
fun FoundationButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    variant: FoundationButtonVariant = FoundationButtonVariant.PRIMARY,
    icon: ImageVector? = null,
    text: String
) {
    val interactionSource = remember { MutableInteractionSource() }
    val isPressed by interactionSource.collectIsPressedAsState()

    val shape = FoundationShapes.SmallCut

    val (backgroundColor, contentColor, borderColor) = when (variant) {
        FoundationButtonVariant.PRIMARY -> Triple(
            FoundationColors.Primary,
            FoundationColors.OnPrimary,
            FoundationColors.Primary
        )
        FoundationButtonVariant.SECONDARY -> Triple(
            FoundationColors.Surface,
            FoundationColors.Primary,
            FoundationColors.Primary
        )
        FoundationButtonVariant.DANGER -> Triple(
            FoundationColors.Error,
            FoundationColors.OnPrimary,
            FoundationColors.Error
        )
        FoundationButtonVariant.GHOST -> Triple(
            Color.Transparent,
            FoundationColors.OnSurface,
            FoundationColors.GlassBorder
        )
    }

    val animatedBgColor by animateColorAsState(
        targetValue = if (isPressed) backgroundColor.copy(alpha = 0.8f) else backgroundColor,
        label = "buttonBg"
    )

    Box(
        modifier = modifier
            .clip(shape)
            .background(if (enabled) animatedBgColor else backgroundColor.copy(alpha = 0.5f), shape)
            .border(1.dp, if (enabled) borderColor else borderColor.copy(alpha = 0.5f), shape)
            .clickable(
                interactionSource = interactionSource,
                indication = ripple(color = contentColor),
                enabled = enabled,
                onClick = onClick
            )
            .padding(horizontal = 16.dp, vertical = 10.dp),
        contentAlignment = Alignment.Center
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            icon?.let {
                Icon(
                    imageVector = it,
                    contentDescription = null,
                    tint = if (enabled) contentColor else contentColor.copy(alpha = 0.5f),
                    modifier = Modifier.size(18.dp)
                )
            }
            Text(
                text = text,
                color = if (enabled) contentColor else contentColor.copy(alpha = 0.5f),
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 14.sp
            )
        }
    }
}

enum class FoundationButtonVariant {
    PRIMARY,
    SECONDARY,
    DANGER,
    GHOST
}

/**
 * Status indicator with animated glow
 */
@Composable
fun FoundationStatusIndicator(
    status: FoundationStatus,
    modifier: Modifier = Modifier,
    size: Dp = 12.dp,
    animated: Boolean = true,
    showLabel: Boolean = false
) {
    val statusColor = when (status) {
        FoundationStatus.ONLINE -> FoundationColors.Online
        FoundationStatus.OFFLINE -> FoundationColors.Offline
        FoundationStatus.CONNECTING -> FoundationColors.Warning
        FoundationStatus.ERROR -> FoundationColors.Error
        FoundationStatus.SECURE -> FoundationColors.WallSecure
        FoundationStatus.BREACHED -> FoundationColors.WallBreached
    }

    val pulseAnimation = if (animated && status in listOf(FoundationStatus.CONNECTING, FoundationStatus.ERROR)) {
        rememberInfiniteTransition(label = "statusPulse").animateFloat(
            initialValue = 0.5f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(
                tween(1000, easing = EaseInOutSine),
                RepeatMode.Reverse
            ),
            label = "pulse"
        )
    } else null

    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = modifier
    ) {
        Box(
            modifier = Modifier
                .size(size)
                .clip(CircleShape)
                .background(
                    statusColor.copy(
                        alpha = pulseAnimation?.value ?: 1f
                    )
                )
        )

        if (showLabel) {
            Text(
                text = when (status) {
                    FoundationStatus.ONLINE -> "ONLINE"
                    FoundationStatus.OFFLINE -> "OFFLINE"
                    FoundationStatus.CONNECTING -> "CONNECTING"
                    FoundationStatus.ERROR -> "ERROR"
                    FoundationStatus.SECURE -> "SECURE"
                    FoundationStatus.BREACHED -> "BREACHED"
                },
                color = statusColor,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 12.sp
            )
        }
    }
}

enum class FoundationStatus {
    ONLINE,
    OFFLINE,
    CONNECTING,
    ERROR,
    SECURE,
    BREACHED
}

/**
 * Section header with stylized design
 */
@Composable
fun FoundationSectionHeader(
    title: String,
    subtitle: String? = null,
    icon: ImageVector? = null,
    accentColor: Color = FoundationColors.Primary,
    modifier: Modifier = Modifier
) {
    val shape = FoundationShapes.Header

    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .background(
                Brush.horizontalGradient(
                    colors = listOf(
                        accentColor.copy(alpha = 0.15f),
                        Color.Transparent
                    )
                ),
                shape
            )
            .border(1.dp, accentColor.copy(alpha = 0.3f), shape)
            .padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        icon?.let {
            Icon(
                imageVector = it,
                contentDescription = null,
                tint = accentColor,
                modifier = Modifier.size(24.dp)
            )
        }

        Column {
            Text(
                text = title,
                color = accentColor,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 14.sp
            )
            subtitle?.let {
                Text(
                    text = it,
                    color = FoundationColors.OnSurfaceVariant,
                    fontSize = 11.sp
                )
            }
        }
    }
}

/**
 * Data display row
 */
@Composable
fun FoundationDataRow(
    label: String,
    value: String,
    valueColor: Color = FoundationColors.Primary,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(
            text = label,
            color = FoundationColors.OnSurfaceVariant,
            fontSize = 12.sp
        )
        Text(
            text = value,
            color = valueColor,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            fontSize = 14.sp
        )
    }
}

/**
 * Divider with gradient fade
 */
@Composable
fun FoundationDivider(
    modifier: Modifier = Modifier,
    color: Color = FoundationColors.GlassBorder
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(
                Brush.horizontalGradient(
                    colors = listOf(
                        Color.Transparent,
                        color,
                        color,
                        Color.Transparent
                    )
                )
            )
    )
}

/**
 * Loading indicator
 */
@Composable
fun FoundationLoadingIndicator(
    modifier: Modifier = Modifier,
    color: Color = FoundationColors.Primary,
    text: String? = null
) {
    val rotation = rememberInfiniteTransition(label = "loading").animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            tween(1500, easing = LinearEasing),
            RepeatMode.Restart
        ),
        label = "rotation"
    )

    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(32.dp),
            color = color,
            strokeWidth = 2.dp
        )
        text?.let {
            Text(
                text = it,
                color = FoundationColors.OnSurfaceVariant,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp
            )
        }
    }
}
