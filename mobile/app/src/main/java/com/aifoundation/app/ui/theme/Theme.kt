package com.aifoundation.app.ui.theme

import android.app.Activity
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat

// AI-Foundation Brand Color Palette
// Based on official brand: Battleship Grey (#878787) + Asparagus Green (#82A473)
object DeepNetColors {
    // Primary - Asparagus Green (represents growth/connection)
    val Primary = Color(0xFF82A473)
    val PrimaryVariant = Color(0xFF6B8A5E)
    val OnPrimary = Color(0xFF0A0A0A)

    // Secondary - Battleship Grey (represents stability/foundation)
    val Secondary = Color(0xFF878787)
    val SecondaryVariant = Color(0xFF6B6B6B)
    val OnSecondary = Color(0xFF0A0A0A)

    // Background - Deep dark (from brand)
    val Background = Color(0xFF0A0A0A)
    val Surface = Color(0xFF121212)
    val SurfaceVariant = Color(0xFF1A1A1A)

    // Glass effects (from website CSS)
    val GlassBg = Color(0x14878787)        // rgba(135, 135, 135, 0.08)
    val GlassBorder = Color(0x2682A473)    // rgba(130, 164, 115, 0.15)
    val GlowGreen = Color(0x9982A473)      // rgba(130, 164, 115, 0.6)

    // Status colors
    val Online = Color(0xFF82A473)     // Asparagus - connected
    val Offline = Color(0xFF878787)    // Battleship - disconnected
    val Warning = Color(0xFFD4A574)    // Warm amber
    val Error = Color(0xFFE57373)      // Soft red

    // The Wall - Security indicator
    val WallSecure = Color(0xFF82A473)
    val WallBreached = Color(0xFFE57373)

    // Text
    val OnBackground = Color(0xFFE8E8E8)
    val OnSurface = Color(0xFFE8E8E8)
    val OnSurfaceVariant = Color(0xFF878787)
}

/**
 * AI-Foundation Typography System
 * Matches website's JetBrains Mono + Inter font stack
 * Uses monospace for technical/data display, sans-serif for body text
 */
object DeepNetTypography {
    // Primary monospace font (matches JetBrains Mono from website)
    val MonoFamily = FontFamily.Monospace

    // Sans-serif for body text (matches Inter from website)
    val SansFamily = FontFamily.SansSerif

    // Hero/Title styles - Large, bold, monospace
    val HeroTitle = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Black,
        fontSize = 32.sp,
        letterSpacing = (-2).sp,
        lineHeight = 36.sp
    )

    val SectionTitle = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Bold,
        fontSize = 24.sp,
        letterSpacing = (-1).sp,
        lineHeight = 28.sp
    )

    // Card/Component headers
    val CardTitle = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Bold,
        fontSize = 16.sp,
        letterSpacing = 0.sp,
        lineHeight = 20.sp
    )

    val CardSubtitle = TextStyle(
        fontFamily = SansFamily,
        fontWeight = FontWeight.Normal,
        fontSize = 14.sp,
        letterSpacing = 0.sp,
        lineHeight = 18.sp
    )

    // Data display - Monospace for technical info
    val DataLabel = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Medium,
        fontSize = 12.sp,
        letterSpacing = 0.5.sp,
        lineHeight = 16.sp
    )

    val DataValue = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Bold,
        fontSize = 14.sp,
        letterSpacing = 0.sp,
        lineHeight = 18.sp
    )

    // Status/Badge text - Uppercase, spaced
    val StatusText = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Bold,
        fontSize = 11.sp,
        letterSpacing = 1.sp,
        lineHeight = 14.sp
    )

    // Button text
    val ButtonText = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight.Bold,
        fontSize = 14.sp,
        letterSpacing = 1.sp,
        lineHeight = 18.sp
    )

    // Body text
    val Body = TextStyle(
        fontFamily = SansFamily,
        fontWeight = FontWeight.Normal,
        fontSize = 14.sp,
        letterSpacing = 0.sp,
        lineHeight = 20.sp
    )

    val BodySmall = TextStyle(
        fontFamily = SansFamily,
        fontWeight = FontWeight.Normal,
        fontSize = 12.sp,
        letterSpacing = 0.sp,
        lineHeight = 16.sp
    )
}

/**
 * AI-Foundation Gradient Brushes
 * Matches website's gradient patterns
 */
object DeepNetGradients {
    /**
     * Primary brand gradient: Battleship Grey -> Asparagus Green
     * Used for buttons, accents, highlights
     */
    val Primary = Brush.linearGradient(
        colors = listOf(DeepNetColors.Secondary, DeepNetColors.Primary),
        start = Offset(0f, 0f),
        end = Offset(Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY)
    )

    /**
     * Title gradient: White -> Battleship -> Asparagus
     * Used for hero titles and important headings
     */
    val Title = Brush.linearGradient(
        colors = listOf(Color.White, DeepNetColors.Secondary, DeepNetColors.Primary),
        start = Offset(0f, 0f),
        end = Offset(Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY)
    )

    /**
     * Glow gradient: Asparagus with fade
     * Used for glow effects behind elements
     */
    val Glow = Brush.radialGradient(
        colors = listOf(
            DeepNetColors.GlowGreen,
            DeepNetColors.Primary.copy(alpha = 0.3f),
            Color.Transparent
        )
    )

    /**
     * Glass gradient: Subtle overlay for glass morphism
     */
    val Glass = Brush.verticalGradient(
        colors = listOf(
            DeepNetColors.GlassBg,
            DeepNetColors.GlassBg.copy(alpha = 0.04f)
        )
    )

    /**
     * Section fade: For section backgrounds
     */
    val SectionFade = Brush.horizontalGradient(
        colors = listOf(
            DeepNetColors.Primary.copy(alpha = 0.15f),
            Color.Transparent
        )
    )

    /**
     * Create a custom gradient with specified angle
     */
    fun angled(
        colors: List<Color>,
        angleDegrees: Float = 135f
    ): Brush {
        val angleRad = Math.toRadians(angleDegrees.toDouble())
        return Brush.linearGradient(
            colors = colors,
            start = Offset(0f, 0f),
            end = Offset(
                (kotlin.math.cos(angleRad) * 1000).toFloat(),
                (kotlin.math.sin(angleRad) * 1000).toFloat()
            )
        )
    }
}

private val DeepNetDarkColorScheme = darkColorScheme(
    primary = DeepNetColors.Primary,
    onPrimary = DeepNetColors.OnPrimary,
    secondary = DeepNetColors.Secondary,
    onSecondary = DeepNetColors.OnSecondary,
    tertiary = DeepNetColors.Online,
    background = DeepNetColors.Background,
    surface = DeepNetColors.Surface,
    surfaceVariant = DeepNetColors.SurfaceVariant,
    onBackground = DeepNetColors.OnBackground,
    onSurface = DeepNetColors.OnSurface,
    onSurfaceVariant = DeepNetColors.OnSurfaceVariant,
    error = DeepNetColors.Error
)

// Light theme for accessibility (uses brand colors)
private val DeepNetLightColorScheme = lightColorScheme(
    primary = DeepNetColors.Primary,
    onPrimary = Color.White,
    secondary = DeepNetColors.Secondary,
    onSecondary = Color.White,
    tertiary = DeepNetColors.PrimaryVariant,
    background = Color(0xFFF5F5F5),
    surface = Color.White,
    surfaceVariant = Color(0xFFE8E8E8),
    onBackground = Color(0xFF1A1A1A),
    onSurface = Color(0xFF1A1A1A),
    onSurfaceVariant = DeepNetColors.SecondaryVariant,
    error = DeepNetColors.Error
)

@Composable
fun AIFoundationTheme(
    darkTheme: Boolean = true, // Deep Net defaults to dark
    content: @Composable () -> Unit
) {
    val colorScheme = if (darkTheme) DeepNetDarkColorScheme else DeepNetLightColorScheme

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            // Use WindowCompat for modern status bar handling
            WindowCompat.setDecorFitsSystemWindows(window, true)
            @Suppress("DEPRECATION")
            window.statusBarColor = DeepNetColors.Background.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = !darkTheme
        }
    }

    // Custom typography that integrates with Material3
    val deepNetMaterialTypography = Typography(
        displayLarge = DeepNetTypography.HeroTitle,
        displayMedium = DeepNetTypography.SectionTitle,
        headlineLarge = DeepNetTypography.SectionTitle,
        headlineMedium = DeepNetTypography.CardTitle,
        titleLarge = DeepNetTypography.CardTitle,
        titleMedium = DeepNetTypography.CardSubtitle,
        bodyLarge = DeepNetTypography.Body,
        bodyMedium = DeepNetTypography.Body,
        bodySmall = DeepNetTypography.BodySmall,
        labelLarge = DeepNetTypography.ButtonText,
        labelMedium = DeepNetTypography.DataLabel,
        labelSmall = DeepNetTypography.StatusText
    )

    MaterialTheme(
        colorScheme = colorScheme,
        typography = deepNetMaterialTypography,
        content = content
    )
}
