package com.aifoundation.app.ui.screens

import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.aifoundation.app.ui.components.*
import com.aifoundation.app.ui.theme.DeepNetColors
import kotlinx.coroutines.delay

/**
 * First-time setup screen shown on initial app launch
 * Handles ID generation, encryption setup, and local storage initialization
 */
@Composable
fun OnboardingScreen(
    onSetupComplete: (nodeId: String) -> Unit
) {
    var setupPhase by remember { mutableStateOf(SetupPhase.WELCOME) }
    var generatedNodeId by remember { mutableStateOf("") }
    var setupProgress by remember { mutableFloatStateOf(0f) }

    LaunchedEffect(setupPhase) {
        when (setupPhase) {
            SetupPhase.GENERATING_IDENTITY -> {
                // Simulate key generation (will be replaced with actual Rust call)
                for (i in 1..100) {
                    setupProgress = i / 100f
                    delay(20)
                }
                // Generate a temporary ID (will use Ed25519 from deepnet-mobile)
                generatedNodeId = generateTemporaryNodeId()
                setupPhase = SetupPhase.INITIALIZING_STORAGE
            }
            SetupPhase.INITIALIZING_STORAGE -> {
                setupProgress = 0f
                for (i in 1..100) {
                    setupProgress = i / 100f
                    delay(15)
                }
                setupPhase = SetupPhase.COMPLETE
            }
            else -> {}
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
            .padding(24.dp),
        contentAlignment = Alignment.Center
    ) {
        when (setupPhase) {
            SetupPhase.WELCOME -> WelcomeContent(
                onBeginSetup = { setupPhase = SetupPhase.GENERATING_IDENTITY }
            )
            SetupPhase.GENERATING_IDENTITY -> SetupProgressContent(
                title = "GENERATING IDENTITY",
                description = "Creating your secure Ed25519 keypair...",
                progress = setupProgress
            )
            SetupPhase.INITIALIZING_STORAGE -> SetupProgressContent(
                title = "INITIALIZING STORAGE",
                description = "Setting up encrypted local database...",
                progress = setupProgress
            )
            SetupPhase.COMPLETE -> SetupCompleteContent(
                nodeId = generatedNodeId,
                onContinue = { onSetupComplete(generatedNodeId) }
            )
        }
    }
}

@Composable
private fun WelcomeContent(onBeginSetup: () -> Unit) {
    val headerShape = DeepNetShapes.Header

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(24.dp)
    ) {
        // Logo/Title area
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .deepNetCornerBrackets(
                    enabled = true,
                    bracketColor = DeepNetColors.Primary,
                    bracketLength = 24.dp,
                    strokeWidth = 2.dp,
                    animated = true
                )
                .padding(32.dp),
            contentAlignment = Alignment.Center
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    text = "DEEP NET",
                    fontSize = 36.sp,
                    fontWeight = FontWeight.Bold,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.Primary
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = "Mobile Client",
                    fontSize = 16.sp,
                    fontFamily = FontFamily.Monospace,
                    color = DeepNetColors.OnSurfaceVariant
                )
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        // Description
        DeepNetCard(
            modifier = Modifier.fillMaxWidth(),
            variant = DeepNetCardVariant.TERMINAL
        ) {
            Text(
                text = "First-time setup will:",
                color = DeepNetColors.OnSurface,
                fontFamily = FontFamily.Monospace,
                fontSize = 14.sp
            )
            Spacer(modifier = Modifier.height(12.dp))

            SetupStepItem(
                number = "01",
                text = "Generate your unique node identity"
            )
            Spacer(modifier = Modifier.height(8.dp))
            SetupStepItem(
                number = "02",
                text = "Create encrypted local storage"
            )
            Spacer(modifier = Modifier.height(8.dp))
            SetupStepItem(
                number = "03",
                text = "Initialize secure messaging"
            )
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Begin button
        DeepNetButton(
            onClick = onBeginSetup,
            variant = DeepNetButtonVariant.PRIMARY,
            icon = Icons.Default.PlayArrow,
            text = "BEGIN SETUP",
            modifier = Modifier.fillMaxWidth()
        )
    }
}

@Composable
private fun SetupStepItem(number: String, text: String) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Text(
            text = number,
            color = DeepNetColors.Primary,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            fontSize = 12.sp
        )
        Text(
            text = text,
            color = DeepNetColors.OnSurfaceVariant,
            fontSize = 13.sp
        )
    }
}

@Composable
private fun SetupProgressContent(
    title: String,
    description: String,
    progress: Float
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(24.dp),
        modifier = Modifier.fillMaxWidth()
    ) {
        // Animated brackets around progress
        Box(
            modifier = Modifier
                .size(120.dp)
                .deepNetCornerBrackets(
                    enabled = true,
                    bracketColor = DeepNetColors.Primary,
                    bracketLength = 20.dp,
                    strokeWidth = 2.dp,
                    animated = true
                ),
            contentAlignment = Alignment.Center
        ) {
            CircularProgressIndicator(
                progress = { progress },
                modifier = Modifier.size(80.dp),
                color = DeepNetColors.Primary,
                trackColor = DeepNetColors.Surface,
                strokeWidth = 4.dp
            )
            Text(
                text = "${(progress * 100).toInt()}%",
                color = DeepNetColors.Primary,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 18.sp
            )
        }

        Text(
            text = title,
            color = DeepNetColors.Primary,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            fontSize = 18.sp
        )

        Text(
            text = description,
            color = DeepNetColors.OnSurfaceVariant,
            fontSize = 14.sp,
            textAlign = TextAlign.Center
        )
    }
}

@Composable
private fun SetupCompleteContent(
    nodeId: String,
    onContinue: () -> Unit
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(24.dp),
        modifier = Modifier.fillMaxWidth()
    ) {
        // Success indicator
        Box(
            modifier = Modifier
                .size(100.dp)
                .deepNetCornerBrackets(
                    enabled = true,
                    bracketColor = DeepNetColors.Online,
                    bracketLength = 16.dp,
                    strokeWidth = 2.dp,
                    animated = false
                ),
            contentAlignment = Alignment.Center
        ) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = null,
                tint = DeepNetColors.Online,
                modifier = Modifier.size(48.dp)
            )
        }

        Text(
            text = "SETUP COMPLETE",
            color = DeepNetColors.Online,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            fontSize = 20.sp
        )

        // Node ID display
        DeepNetCard(
            modifier = Modifier.fillMaxWidth(),
            variant = DeepNetCardVariant.NODE,
            enableGlow = true
        ) {
            Text(
                text = "Your Node ID",
                color = DeepNetColors.OnSurfaceVariant,
                fontSize = 12.sp
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = nodeId,
                color = DeepNetColors.Primary,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
                fontSize = 16.sp
            )
        }

        Text(
            text = "This ID uniquely identifies your device on the network.",
            color = DeepNetColors.OnSurfaceVariant,
            fontSize = 13.sp,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(16.dp))

        DeepNetButton(
            onClick = onContinue,
            variant = DeepNetButtonVariant.PRIMARY,
            icon = Icons.AutoMirrored.Filled.ArrowForward,
            text = "CONTINUE",
            modifier = Modifier.fillMaxWidth()
        )
    }
}

private enum class SetupPhase {
    WELCOME,
    GENERATING_IDENTITY,
    INITIALIZING_STORAGE,
    COMPLETE
}

/**
 * Temporary node ID generator
 * Will be replaced with actual Ed25519 key generation from deepnet-mobile Rust lib
 */
private fun generateTemporaryNodeId(): String {
    val adjectives = listOf("swift", "bright", "calm", "deep", "keen", "bold", "wise", "true")
    val nouns = listOf("node", "link", "core", "beam", "wave", "flux", "grid", "mesh")
    val adj = adjectives.random()
    val noun = nouns.random()
    val num = (100..999).random()
    return "$adj-$noun-$num"
}
