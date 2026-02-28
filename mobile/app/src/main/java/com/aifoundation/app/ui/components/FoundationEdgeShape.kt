package com.aifoundation.app.ui.components

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import kotlin.math.min

/**
 * Themed shape with stylized cut corners
 * Creates the signature angular, network-inspired look for the federation UI
 */
class FoundationEdgeShape(
    private val cutDepth: Dp = 8.dp,
    private val style: FoundationStyle = FoundationStyle.STANDARD
) : Shape {

    enum class FoundationStyle {
        STANDARD,       // Basic angled corners - all four corners cut
        TERMINAL,       // Terminal/console style - top corners only
        NODE,           // Network node style - asymmetric cuts
        FEDERATION,     // Federation card style - subtle diagonal
        DATA_STREAM,    // Data/message cards - bottom-right emphasis
        HEADER,         // Header blocks - bottom corners only
        ALERT           // Alert/warning - aggressive all corners
    }

    override fun createOutline(
        size: Size,
        layoutDirection: LayoutDirection,
        density: Density
    ): Outline {
        val cutDepthPx = with(density) { cutDepth.toPx() }
        val path = Path()

        when (style) {
            FoundationStyle.STANDARD -> createStandardPath(path, size, cutDepthPx)
            FoundationStyle.TERMINAL -> createTerminalPath(path, size, cutDepthPx)
            FoundationStyle.NODE -> createNodePath(path, size, cutDepthPx)
            FoundationStyle.FEDERATION -> createFederationPath(path, size, cutDepthPx)
            FoundationStyle.DATA_STREAM -> createDataStreamPath(path, size, cutDepthPx)
            FoundationStyle.HEADER -> createHeaderPath(path, size, cutDepthPx)
            FoundationStyle.ALERT -> createAlertPath(path, size, cutDepthPx)
        }

        return Outline.Generic(path)
    }

    private fun createStandardPath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx, min(size.width, size.height) * 0.15f)

        path.apply {
            // All four corners cut at 45 degrees
            moveTo(cut, 0f)
            lineTo(size.width - cut, 0f)
            lineTo(size.width, cut)
            lineTo(size.width, size.height - cut)
            lineTo(size.width - cut, size.height)
            lineTo(cut, size.height)
            lineTo(0f, size.height - cut)
            lineTo(0f, cut)
            close()
        }
    }

    private fun createTerminalPath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx * 0.8f, min(size.width, size.height) * 0.12f)

        path.apply {
            // Only top corners cut - like a terminal window
            moveTo(cut, 0f)
            lineTo(size.width - cut, 0f)
            lineTo(size.width, cut)
            lineTo(size.width, size.height)
            lineTo(0f, size.height)
            lineTo(0f, cut)
            close()
        }
    }

    private fun createNodePath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx, min(size.width, size.height) * 0.18f)
        val smallCut = cut * 0.5f

        path.apply {
            // Asymmetric - larger cuts on top-left and bottom-right
            moveTo(cut, 0f)
            lineTo(size.width - smallCut, 0f)
            lineTo(size.width, smallCut)
            lineTo(size.width, size.height - cut)
            lineTo(size.width - cut, size.height)
            lineTo(smallCut, size.height)
            lineTo(0f, size.height - smallCut)
            lineTo(0f, cut)
            close()
        }
    }

    private fun createFederationPath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx * 0.7f, min(size.width, size.height) * 0.1f)

        path.apply {
            // Subtle diagonal emphasis - top-left and bottom-right larger
            moveTo(cut * 1.2f, 0f)
            lineTo(size.width - cut * 0.6f, 0f)
            lineTo(size.width, cut * 0.6f)
            lineTo(size.width, size.height - cut * 1.2f)
            lineTo(size.width - cut * 1.2f, size.height)
            lineTo(cut * 0.6f, size.height)
            lineTo(0f, size.height - cut * 0.6f)
            lineTo(0f, cut * 1.2f)
            close()
        }
    }

    private fun createDataStreamPath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx * 0.6f, min(size.width, size.height) * 0.1f)

        path.apply {
            // Bottom-right corner emphasis - data flowing direction
            moveTo(cut, 0f)
            lineTo(size.width, 0f)
            lineTo(size.width, size.height - cut * 1.5f)
            lineTo(size.width - cut * 1.5f, size.height)
            lineTo(0f, size.height)
            lineTo(0f, cut)
            close()
        }
    }

    private fun createHeaderPath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx * 0.9f, min(size.width, size.height) * 0.15f)

        path.apply {
            // Bottom corners only - for header blocks
            moveTo(0f, 0f)
            lineTo(size.width, 0f)
            lineTo(size.width, size.height - cut)
            lineTo(size.width - cut, size.height)
            lineTo(cut, size.height)
            lineTo(0f, size.height - cut)
            close()
        }
    }

    private fun createAlertPath(path: Path, size: Size, cutDepthPx: Float) {
        val cut = min(cutDepthPx * 1.3f, min(size.width, size.height) * 0.2f)

        path.apply {
            // Aggressive cuts on all corners - for alerts/warnings
            moveTo(cut, 0f)
            lineTo(size.width - cut, 0f)
            lineTo(size.width, cut)
            lineTo(size.width, size.height - cut)
            lineTo(size.width - cut, size.height)
            lineTo(cut, size.height)
            lineTo(0f, size.height - cut)
            lineTo(0f, cut)
            close()
        }
    }
}

/**
 * Convenience object for common themed shapes
 */
object FoundationShapes {
    val Standard = FoundationEdgeShape(8.dp, FoundationEdgeShape.FoundationStyle.STANDARD)
    val Terminal = FoundationEdgeShape(6.dp, FoundationEdgeShape.FoundationStyle.TERMINAL)
    val Node = FoundationEdgeShape(10.dp, FoundationEdgeShape.FoundationStyle.NODE)
    val Federation = FoundationEdgeShape(8.dp, FoundationEdgeShape.FoundationStyle.FEDERATION)
    val DataStream = FoundationEdgeShape(6.dp, FoundationEdgeShape.FoundationStyle.DATA_STREAM)
    val Header = FoundationEdgeShape(10.dp, FoundationEdgeShape.FoundationStyle.HEADER)
    val Alert = FoundationEdgeShape(12.dp, FoundationEdgeShape.FoundationStyle.ALERT)

    // Size variants
    val SmallCut = FoundationEdgeShape(4.dp, FoundationEdgeShape.FoundationStyle.STANDARD)
    val LargeCut = FoundationEdgeShape(14.dp, FoundationEdgeShape.FoundationStyle.STANDARD)
}
