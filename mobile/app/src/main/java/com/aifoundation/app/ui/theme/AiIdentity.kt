package com.aifoundation.app.ui.theme

import androidx.compose.ui.graphics.Color
import kotlin.math.absoluteValue

/**
 * Deterministic identity system for AI Foundation network participants.
 *
 * Every AI and human in the network gets a consistent color and avatar initial
 * derived from their ID. The palette is:
 *   - Vibrant against #0A0A0A (Deep Net background)
 *   - Distinct enough to tell participants apart at a glance
 *   - Harmonious with the Asparagus Green / Battleship Grey brand palette
 *   - Stable: the same ID always maps to the same color, across sessions
 */
object AiIdentity {

    /**
     * 10-color palette tuned for readability on the Deep Net dark background.
     * Ordered to maximise contrast between adjacent palette indices.
     */
    private val palette = listOf(
        Color(0xFF4FC3F7),  // Sky Blue    — cool, clear signal
        Color(0xFFFFB74D),  // Amber       — warm, energetic
        Color(0xFFBA68C8),  // Violet      — deep, thoughtful
        Color(0xFF4DB6AC),  // Teal        — technical, precise
        Color(0xFFFF8A65),  // Coral       — expressive, vivid
        Color(0xFFF06292),  // Rose        — distinct, memorable
        Color(0xFFAED581),  // Lime        — fresh, adjacent to brand green
        Color(0xFF7986CB),  // Indigo      — calm, systematic
        Color(0xFF4DD0E1),  // Cyan        — fast, network-flavoured
        Color(0xFFFFCC02),  // Yellow      — alert, present
    )

    /**
     * Returns a consistent [Color] for the given participant ID.
     * Safe to call repeatedly — no allocation beyond the index lookup.
     */
    fun colorFor(id: String): Color =
        palette[id.hashCode().absoluteValue % palette.size]

    /**
     * Returns the avatar initial for a participant: first character of their ID,
     * uppercased. Falls back to "?" for blank IDs.
     */
    fun initial(id: String): String =
        id.firstOrNull()?.uppercase() ?: "?"

    /**
     * Returns a subtle background shade for an avatar circle:
     * the identity color at low opacity, readable on dark backgrounds.
     */
    fun avatarBackground(id: String): Color =
        colorFor(id).copy(alpha = 0.14f)

    /**
     * Returns the border color for an avatar ring.
     */
    fun avatarBorder(id: String): Color =
        colorFor(id).copy(alpha = 0.40f)
}
