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
import com.aifoundation.app.ui.theme.DeepNetColors

/**
 * Deep Net themed UI components
 * Network-inspired visual design for the federation UI
 */

/**
 * Card variant types for different contexts
 */
enum class DeepNetCardVariant {
    STANDARD,       // Default card style
    NODE,           // For network node display
    TERMINAL,       // Terminal/console style
    DATA,           // Data display cards
    FEDERATION,     // Federation-related content
    ALERT,          // Alerts and warnings
    SUCCESS         // Success/confirmation
}

/**
 * Deep Net styled card with cut corners and optional effects
 */
@Composable
fun DeepNetCard(
    modifier: Modifier = Modifier,
    variant: DeepNetCardVariant = DeepNetCardVariant.STANDARD,
    onClick: (() -> Unit)? = null,
    enablePulse: Boolean = false,
    enableGlow: Boolean = false,
    enableBrackets: Boolean = false,
    content: @Composable ColumnScope.() -> Unit
) {
    val shape = when (variant) {
        DeepNetCardVariant.STANDARD -> DeepNetShapes.Standard
        DeepNetCardVariant.NODE -> DeepNetShapes.Node
        DeepNetCardVariant.TERMINAL -> DeepNetShapes.Terminal
        DeepNetCardVariant.DATA -> DeepNetShapes.DataStream
        DeepNetCardVariant.FEDERATION -> DeepNetShapes.Federation
        DeepNetCardVariant.ALERT -> DeepNetShapes.Alert
        DeepNetCardVariant.SUCCESS -> DeepNetShapes.Standard
    }

    // All cards get visible asparagus green border by default
    val (backgroundColor, borderColor) = when (variant) {
        DeepNetCardVariant.STANDARD -> DeepNetColors.Surface to DeepNetColors.Primary.copy(alpha = 0.6f)
        DeepNetCardVariant.NODE -> DeepNetColors.Surface to DeepNetColors.Primary.copy(alpha = 0.7f)
        DeepNetCardVariant.TERMINAL -> DeepNetColors.Background to DeepNetColors.Primary.copy(alpha = 0.8f)
        DeepNetCardVariant.DATA -> DeepNetColors.Surface to DeepNetColors.Primary.copy(alpha = 0.5f)
        DeepNetCardVariant.FEDERATION -> DeepNetColors.Surface to DeepNetColors.Primary.copy(alpha = 0.6f)
        DeepNetCardVariant.ALERT -> DeepNetColors.Surface to DeepNetColors.Error.copy(alpha = 0.7f)
        DeepNetCardVariant.SUCCESS -> DeepNetColors.Surface to DeepNetColors.Online.copy(alpha = 0.7f)
    }

    val pulseColor = when (variant) {
        DeepNetCardVariant.ALERT -> DeepNetColors.Error
        DeepNetCardVariant.SUCCESS -> DeepNetColors.Online
        else -> DeepNetColors.Primary
    }

    var cardModifier = modifier
        .clip(shape)
        .background(backgroundColor, shape)
        .border(1.dp, borderColor, shape)

    if (enablePulse) {
        cardModifier = cardModifier.deepNetEnergyPulse(
            enabled = true,
            pulseColor = pulseColor,
            shape = shape
        )
    }

    if (enableGlow) {
        cardModifier = cardModifier.deepNetGlow(
            enabled = true,
            glowColor = pulseColor,
            shape = shape
        )
    }

    if (enableBrackets) {
        cardModifier = cardModifier.deepNetCornerBrackets(
            enabled = true,
            bracketColor = borderColor
        )
    }

    if (onClick != null) {
        cardModifier = cardModifier.clickable(
            interactionSource = remember { MutableInteractionSource() },
            indication = ripple(color = DeepNetColors.Primary),
            onClick = onClick
        )
    }

    Column(
        modifier = cardModifier.padding(12.dp),
        content = content
    )
}

/**
 * Deep Net styled button with cut corners
 */
@Composable
fun DeepNetButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    variant: DeepNetButtonVariant = DeepNetButtonVariant.PRIMARY,
    icon: ImageVector? = null,
    text: String
) {
    val interactionSource = remember { MutableInteractionSource() }
    val isPressed by interactionSource.collectIsPressedAsState()

    val shape = DeepNetShapes.SmallCut

    val (backgroundColor, contentColor, borderColor) = when (variant) {
        DeepNetButtonVariant.PRIMARY -> Triple(
            DeepNetColors.Primary,
            DeepNetColors.OnPrimary,
            DeepNetColors.Primary
        )
        DeepNetButtonVariant.SECONDARY -> Triple(
            DeepNetColors.Surface,
            DeepNetColors.Primary,
            DeepNetColors.Primary
        )
        DeepNetButtonVariant.DANGER -> Triple(
            DeepNetColors.Error,
            DeepNetColors.OnPrimary,
            DeepNetColors.Error
        )
        DeepNetButtonVariant.GHOST -> Triple(
            Color.Transparent,
            DeepNetColors.OnSurface,
            DeepNetColors.GlassBorder
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

enum class DeepNetButtonVariant {
    PRIMARY,
    SECONDARY,
    DANGER,
    GHOST
}

/**
 * Deep Net status indicator with animated glow
 */
@Composable
fun DeepNetStatusIndicator(
    status: DeepNetStatus,
    modifier: Modifier = Modifier,
    size: Dp = 12.dp,
    animated: Boolean = true,
    showLabel: Boolean = false
) {
    val statusColor = when (status) {
        DeepNetStatus.ONLINE -> DeepNetColors.Online
        DeepNetStatus.OFFLINE -> DeepNetColors.Offline
        DeepNetStatus.CONNECTING -> DeepNetColors.Warning
        DeepNetStatus.ERROR -> DeepNetColors.Error
        DeepNetStatus.SECURE -> DeepNetColors.WallSecure
        DeepNetStatus.BREACHED -> DeepNetColors.WallBreached
    }

    val pulseAnimation = if (animated && status in listOf(DeepNetStatus.CONNECTING, DeepNetStatus.ERROR)) {
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
                    DeepNetStatus.ONLINE -> "ONLINE"
                    DeepNetStatus.OFFLINE -> "OFFLINE"
                    DeepNetStatus.CONNECTING -> "CONNECTING"
                    DeepNetStatus.ERROR -> "ERROR"
                    DeepNetStatus.SECURE -> "SECURE"
                    DeepNetStatus.BREACHED -> "BREACHED"
                },
                color = statusColor,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 12.sp
            )
        }
    }
}

enum class DeepNetStatus {
    ONLINE,
    OFFLINE,
    CONNECTING,
    ERROR,
    SECURE,
    BREACHED
}

/**
 * Deep Net section header with stylized design
 */
@Composable
fun DeepNetSectionHeader(
    title: String,
    subtitle: String? = null,
    icon: ImageVector? = null,
    accentColor: Color = DeepNetColors.Primary,
    modifier: Modifier = Modifier
) {
    val shape = DeepNetShapes.Header

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
                    color = DeepNetColors.OnSurfaceVariant,
                    fontSize = 11.sp
                )
            }
        }
    }
}

/**
 * Deep Net data display row
 */
@Composable
fun DeepNetDataRow(
    label: String,
    value: String,
    valueColor: Color = DeepNetColors.Primary,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(
            text = label,
            color = DeepNetColors.OnSurfaceVariant,
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
 * Deep Net divider with gradient fade
 */
@Composable
fun DeepNetDivider(
    modifier: Modifier = Modifier,
    color: Color = DeepNetColors.GlassBorder
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
 * Deep Net loading indicator
 */
@Composable
fun DeepNetLoadingIndicator(
    modifier: Modifier = Modifier,
    color: Color = DeepNetColors.Primary,
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
                color = DeepNetColors.OnSurfaceVariant,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp
            )
        }
    }
}
