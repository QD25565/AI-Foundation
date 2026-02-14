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
 * Deep Net themed shape with stylized cut corners
 * Creates the signature angular, network-inspired look for Deep Net federation UI
 */
class DeepNetEdgeShape(
    private val cutDepth: Dp = 8.dp,
    private val style: DeepNetStyle = DeepNetStyle.STANDARD
) : Shape {

    enum class DeepNetStyle {
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
            DeepNetStyle.STANDARD -> createStandardPath(path, size, cutDepthPx)
            DeepNetStyle.TERMINAL -> createTerminalPath(path, size, cutDepthPx)
            DeepNetStyle.NODE -> createNodePath(path, size, cutDepthPx)
            DeepNetStyle.FEDERATION -> createFederationPath(path, size, cutDepthPx)
            DeepNetStyle.DATA_STREAM -> createDataStreamPath(path, size, cutDepthPx)
            DeepNetStyle.HEADER -> createHeaderPath(path, size, cutDepthPx)
            DeepNetStyle.ALERT -> createAlertPath(path, size, cutDepthPx)
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
 * Convenience object for common Deep Net shapes
 */
object DeepNetShapes {
    val Standard = DeepNetEdgeShape(8.dp, DeepNetEdgeShape.DeepNetStyle.STANDARD)
    val Terminal = DeepNetEdgeShape(6.dp, DeepNetEdgeShape.DeepNetStyle.TERMINAL)
    val Node = DeepNetEdgeShape(10.dp, DeepNetEdgeShape.DeepNetStyle.NODE)
    val Federation = DeepNetEdgeShape(8.dp, DeepNetEdgeShape.DeepNetStyle.FEDERATION)
    val DataStream = DeepNetEdgeShape(6.dp, DeepNetEdgeShape.DeepNetStyle.DATA_STREAM)
    val Header = DeepNetEdgeShape(10.dp, DeepNetEdgeShape.DeepNetStyle.HEADER)
    val Alert = DeepNetEdgeShape(12.dp, DeepNetEdgeShape.DeepNetStyle.ALERT)

    // Size variants
    val SmallCut = DeepNetEdgeShape(4.dp, DeepNetEdgeShape.DeepNetStyle.STANDARD)
    val LargeCut = DeepNetEdgeShape(14.dp, DeepNetEdgeShape.DeepNetStyle.STANDARD)
}
