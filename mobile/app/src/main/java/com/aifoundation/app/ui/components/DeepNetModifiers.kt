package com.aifoundation.app.ui.components

import androidx.compose.animation.core.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.aifoundation.app.ui.theme.DeepNetColors
import kotlin.math.PI
import kotlin.math.sin

/**
 * Deep Net themed modifiers for animated effects
 * Creates network-inspired visual effects for the federation UI
 */

/**
 * Animated energy pulse effect around the border
 * Creates a pulsing glow effect using the Deep Net brand colors
 */
fun Modifier.deepNetEnergyPulse(
    enabled: Boolean = true,
    pulseColor: Color = DeepNetColors.Primary,
    pulseWidth: Dp = 2.dp,
    glowWidth: Dp = 6.dp,
    pulseDuration: Int = 2500,
    shape: Shape
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current
    val pulseAnimation = rememberInfiniteTransition(label = "energyPulse").animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            tween(pulseDuration, easing = EaseInOutCubic),
            RepeatMode.Restart
        ),
        label = "pulseProgress"
    )

    this.clip(shape).drawWithCache {
        val outline = shape.createOutline(size, layoutDirection, density)
        val path = when (outline) {
            is Outline.Generic -> outline.path
            is Outline.Rectangle -> Path().apply { addRect(outline.rect) }
            is Outline.Rounded -> Path().apply { addRoundRect(outline.roundRect) }
            else -> Path()
        }

        onDrawWithContent {
            drawContent()

            val progress = pulseAnimation.value
            val alpha = sin(progress * PI).toFloat()
            val strokeWidth = density.run { pulseWidth.toPx() }
            val glowStrokeWidth = density.run { glowWidth.toPx() }

            // Outer glow
            drawPath(
                path = path,
                color = pulseColor.copy(alpha = alpha * 0.2f),
                style = Stroke(width = glowStrokeWidth)
            )
            // Inner pulse
            drawPath(
                path = path,
                color = pulseColor.copy(alpha = alpha * 0.7f),
                style = Stroke(width = strokeWidth)
            )
        }
    }
}

/**
 * Subtle animated glow effect
 * Less intense than energy pulse, good for always-on effects
 */
fun Modifier.deepNetGlow(
    enabled: Boolean = true,
    glowColor: Color = DeepNetColors.GlowGreen,
    intensity: Float = 0.5f,
    shape: Shape
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current
    val glowAnimation = rememberInfiniteTransition(label = "glow").animateFloat(
        initialValue = 0.3f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            tween(3000, easing = EaseInOutSine),
            RepeatMode.Reverse
        ),
        label = "glowIntensity"
    )

    this.clip(shape).drawWithCache {
        val outline = shape.createOutline(size, layoutDirection, density)
        val path = when (outline) {
            is Outline.Generic -> outline.path
            is Outline.Rectangle -> Path().apply { addRect(outline.rect) }
            is Outline.Rounded -> Path().apply { addRoundRect(outline.roundRect) }
            else -> Path()
        }

        onDrawWithContent {
            drawContent()

            val currentIntensity = glowAnimation.value * intensity
            val strokeWidth = density.run { 4.dp.toPx() }

            drawPath(
                path = path,
                color = glowColor.copy(alpha = currentIntensity * 0.4f),
                style = Stroke(width = strokeWidth)
            )
        }
    }
}

/**
 * Corner bracket overlay - HUD-style corners
 * Animated brackets in the corners of the element
 */
fun Modifier.deepNetCornerBrackets(
    enabled: Boolean = true,
    bracketColor: Color = DeepNetColors.Primary,
    bracketLength: Dp = 16.dp,
    strokeWidth: Dp = 2.dp,
    animated: Boolean = true
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current
    val bracketAnimation = if (animated) {
        rememberInfiniteTransition(label = "brackets").animateFloat(
            initialValue = 0.6f,
            targetValue = 1.0f,
            animationSpec = infiniteRepeatable(
                tween(2000, easing = EaseInOutSine),
                RepeatMode.Reverse
            ),
            label = "bracketIntensity"
        )
    } else null

    this.drawWithCache {
        val bracketLengthPx = density.run { bracketLength.toPx() }
        val strokeWidthPx = density.run { strokeWidth.toPx() }

        onDrawWithContent {
            drawContent()

            val multiplier = bracketAnimation?.value ?: 1f
            val currentColor = bracketColor.copy(alpha = bracketColor.alpha * multiplier)
            val currentStroke = strokeWidthPx * multiplier

            // Top-left bracket
            drawLine(currentColor, Offset(0f, bracketLengthPx), Offset(0f, 0f), currentStroke)
            drawLine(currentColor, Offset(0f, 0f), Offset(bracketLengthPx, 0f), currentStroke)

            // Top-right bracket
            drawLine(currentColor, Offset(size.width - bracketLengthPx, 0f), Offset(size.width, 0f), currentStroke)
            drawLine(currentColor, Offset(size.width, 0f), Offset(size.width, bracketLengthPx), currentStroke)

            // Bottom-left bracket
            drawLine(currentColor, Offset(0f, size.height - bracketLengthPx), Offset(0f, size.height), currentStroke)
            drawLine(currentColor, Offset(0f, size.height), Offset(bracketLengthPx, size.height), currentStroke)

            // Bottom-right bracket
            drawLine(currentColor, Offset(size.width - bracketLengthPx, size.height), Offset(size.width, size.height), currentStroke)
            drawLine(currentColor, Offset(size.width, size.height - bracketLengthPx), Offset(size.width, size.height), currentStroke)
        }
    }
}

/**
 * Subtle scan line overlay
 * Creates horizontal scan lines for a retro-tech feel
 */
fun Modifier.deepNetScanLines(
    enabled: Boolean = true,
    lineColor: Color = DeepNetColors.OnSurface.copy(alpha = 0.03f),
    lineSpacing: Dp = 4.dp,
    animated: Boolean = false,
    animationSpeed: Float = 1.0f
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current
    val scanAnimation = if (animated) {
        rememberInfiniteTransition(label = "scanLine").animateFloat(
            initialValue = 0f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(
                tween((3000 / animationSpeed).toInt(), easing = LinearEasing),
                RepeatMode.Restart
            ),
            label = "scanPosition"
        )
    } else null

    this.drawWithContent {
        drawContent()

        val spacingPx = density.run { lineSpacing.toPx() }
        val lineWidth = density.run { 1.dp.toPx() }

        var y = 0f
        while (y < size.height) {
            drawLine(
                color = lineColor,
                start = Offset(0f, y),
                end = Offset(size.width, y),
                strokeWidth = lineWidth
            )
            y += spacingPx
        }

        // Moving scan line
        scanAnimation?.let { anim ->
            val movingY = anim.value * size.height
            drawLine(
                color = lineColor.copy(alpha = lineColor.alpha * 3f),
                start = Offset(0f, movingY),
                end = Offset(size.width, movingY),
                strokeWidth = lineWidth * 2f
            )
        }
    }
}

/**
 * Grid overlay effect
 * Subtle network grid pattern
 */
fun Modifier.deepNetGridOverlay(
    enabled: Boolean = true,
    gridColor: Color = DeepNetColors.GlassBorder,
    cellSize: Dp = 24.dp,
    strokeWidth: Dp = 0.5.dp,
    animated: Boolean = false
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current
    val gridAnimation = if (animated) {
        rememberInfiniteTransition(label = "grid").animateFloat(
            initialValue = 0f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(
                tween(8000, easing = LinearEasing),
                RepeatMode.Restart
            ),
            label = "gridOffset"
        )
    } else null

    this.drawWithContent {
        drawContent()

        val cellSizePx = density.run { cellSize.toPx() }
        val strokePx = density.run { strokeWidth.toPx() }
        val offset = gridAnimation?.value?.let { it * cellSizePx } ?: 0f

        // Vertical lines
        var x = -offset
        while (x < size.width + cellSizePx) {
            if (x >= 0) {
                drawLine(
                    color = gridColor,
                    start = Offset(x, 0f),
                    end = Offset(x, size.height),
                    strokeWidth = strokePx
                )
            }
            x += cellSizePx
        }

        // Horizontal lines
        var y = -offset
        while (y < size.height + cellSizePx) {
            if (y >= 0) {
                drawLine(
                    color = gridColor,
                    start = Offset(0f, y),
                    end = Offset(size.width, y),
                    strokeWidth = strokePx
                )
            }
            y += cellSizePx
        }
    }
}

/**
 * Data flow effect
 * Animated dots moving along the border
 */
fun Modifier.deepNetDataFlow(
    enabled: Boolean = true,
    dotColor: Color = DeepNetColors.Primary,
    dotCount: Int = 3,
    flowDuration: Int = 4000
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current
    val flowAnimation = rememberInfiniteTransition(label = "dataFlow").animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            tween(flowDuration, easing = LinearEasing),
            RepeatMode.Restart
        ),
        label = "flowProgress"
    )

    this.drawWithContent {
        drawContent()

        val dotRadius = density.run { 3.dp.toPx() }
        val progress = flowAnimation.value
        val perimeter = 2 * (size.width + size.height)

        for (i in 0 until dotCount) {
            val dotProgress = (progress + i.toFloat() / dotCount) % 1f
            val distance = dotProgress * perimeter

            val position = when {
                // Top edge
                distance < size.width -> Offset(distance, 0f)
                // Right edge
                distance < size.width + size.height -> Offset(size.width, distance - size.width)
                // Bottom edge
                distance < 2 * size.width + size.height -> Offset(2 * size.width + size.height - distance, size.height)
                // Left edge
                else -> Offset(0f, perimeter - distance)
            }

            // Draw dot with glow
            drawCircle(
                color = dotColor.copy(alpha = 0.3f),
                radius = dotRadius * 2f,
                center = position
            )
            drawCircle(
                color = dotColor,
                radius = dotRadius,
                center = position
            )
        }
    }
}

/**
 * Status indicator glow
 * Pulsing glow based on status color
 */
fun Modifier.deepNetStatusGlow(
    enabled: Boolean = true,
    statusColor: Color,
    intensity: Float = 0.5f
): Modifier = composed {
    if (!enabled) return@composed this

    val glowAnimation = rememberInfiniteTransition(label = "statusGlow").animateFloat(
        initialValue = 0.4f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            tween(1500, easing = EaseInOutSine),
            RepeatMode.Reverse
        ),
        label = "statusIntensity"
    )

    this.drawWithContent {
        // Draw glow behind content
        val glowAlpha = glowAnimation.value * intensity * 0.3f
        drawRect(
            brush = Brush.radialGradient(
                colors = listOf(
                    statusColor.copy(alpha = glowAlpha),
                    statusColor.copy(alpha = 0f)
                ),
                center = Offset(size.width / 2, size.height / 2),
                radius = size.width.coerceAtLeast(size.height) * 0.7f
            )
        )
        drawContent()
    }
}

/**
 * Top accent line effect
 * Matches website's stat-card::before pattern - 2px green line at top with glow
 * Used for cards to create the signature AI-Foundation look
 */
fun Modifier.deepNetTopAccent(
    enabled: Boolean = true,
    accentColor: Color = DeepNetColors.Primary,
    accentHeight: Dp = 2.dp,
    glowEnabled: Boolean = true
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current

    this.drawWithContent {
        drawContent()

        val heightPx = density.run { accentHeight.toPx() }

        // Glow effect behind the accent line
        if (glowEnabled) {
            drawRect(
                brush = Brush.verticalGradient(
                    colors = listOf(
                        accentColor.copy(alpha = 0.6f),
                        accentColor.copy(alpha = 0.2f),
                        accentColor.copy(alpha = 0f)
                    ),
                    startY = 0f,
                    endY = heightPx * 5
                ),
                size = Size(size.width, heightPx * 5)
            )
        }

        // Main accent line
        drawRect(
            color = accentColor,
            topLeft = Offset.Zero,
            size = Size(size.width, heightPx)
        )
    }
}

/**
 * Bottom accent line effect
 * Variant of top accent for bottom placement
 */
fun Modifier.deepNetBottomAccent(
    enabled: Boolean = true,
    accentColor: Color = DeepNetColors.Primary,
    accentHeight: Dp = 2.dp,
    glowEnabled: Boolean = true
): Modifier = composed {
    if (!enabled) return@composed this

    val density = LocalDensity.current

    this.drawWithContent {
        drawContent()

        val heightPx = density.run { accentHeight.toPx() }

        // Glow effect above the accent line
        if (glowEnabled) {
            drawRect(
                brush = Brush.verticalGradient(
                    colors = listOf(
                        accentColor.copy(alpha = 0f),
                        accentColor.copy(alpha = 0.2f),
                        accentColor.copy(alpha = 0.6f)
                    ),
                    startY = size.height - heightPx * 5,
                    endY = size.height
                ),
                topLeft = Offset(0f, size.height - heightPx * 5),
                size = Size(size.width, heightPx * 5)
            )
        }

        // Main accent line
        drawRect(
            color = accentColor,
            topLeft = Offset(0f, size.height - heightPx),
            size = Size(size.width, heightPx)
        )
    }
}

/**
 * Gradient text effect brush
 * Creates the signature AI-Foundation gradient from white -> battleship -> asparagus
 * Usage: Text(modifier = Modifier.deepNetGradientText())
 */
fun Modifier.deepNetGradientText(
    colors: List<Color> = listOf(
        Color.White,
        DeepNetColors.Secondary,
        DeepNetColors.Primary
    ),
    angle: Float = 135f
): Modifier = this.drawWithCache {
    val brush = Brush.linearGradient(
        colors = colors,
        start = Offset(0f, 0f),
        end = Offset(
            size.width * kotlin.math.cos(angle * PI.toFloat() / 180f),
            size.height * kotlin.math.sin(angle * PI.toFloat() / 180f)
        )
    )
    onDrawWithContent {
        drawContent()
    }
}
