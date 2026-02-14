package com.aifoundation.app.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
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

/**
 * Pairing screen - Enter a pairing code to link this device to an H_ID.
 * The code is generated on the PC via: POST /api/pair/generate
 * This screen sends the code to: POST /api/pair
 */
@Composable
fun PairingScreen(
    serverUrl: String,
    onServerUrlChange: (String) -> Unit,
    onPair: (String, String) -> Unit, // serverUrl, code
    isPairing: Boolean,
    error: String?,
    onClearError: () -> Unit
) {
    var editableUrl by remember(serverUrl) { mutableStateOf(serverUrl) }
    var pairingCode by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DeepNetColors.Background)
            .padding(start = 24.dp, end = 24.dp, top = 24.dp, bottom = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Spacer(modifier = Modifier.height(48.dp))

        // Title
        Text(
            text = "AI-FOUNDATION",
            fontSize = 28.sp,
            fontWeight = FontWeight.Black,
            fontFamily = FontFamily.Monospace,
            color = DeepNetColors.Primary,
            letterSpacing = (-1).sp
        )
        Spacer(modifier = Modifier.height(4.dp))
        Text(
            text = "HUMAN INTERFACE",
            fontSize = 14.sp,
            fontFamily = FontFamily.Monospace,
            color = DeepNetColors.Secondary,
            letterSpacing = 2.sp
        )

        Spacer(modifier = Modifier.height(48.dp))

        // Server URL
        DeepNetCard(
            modifier = Modifier.fillMaxWidth(),
            variant = DeepNetCardVariant.TERMINAL
        ) {
            Text(
                text = "Server URL",
                style = MaterialTheme.typography.labelMedium,
                color = DeepNetColors.OnSurfaceVariant
            )
            Spacer(modifier = Modifier.height(8.dp))
            OutlinedTextField(
                value = editableUrl,
                onValueChange = { editableUrl = it },
                placeholder = { Text("http://192.168.x.x:8080") },
                modifier = Modifier.fillMaxWidth(),
                enabled = !isPairing,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = DeepNetColors.Primary,
                    unfocusedBorderColor = DeepNetColors.OnSurfaceVariant,
                    cursorColor = DeepNetColors.Primary,
                    focusedTextColor = DeepNetColors.OnSurface,
                    unfocusedTextColor = DeepNetColors.OnSurface
                ),
                singleLine = true
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Emulator: 10.0.2.2:8080 | Real device: your PC's IP",
                style = MaterialTheme.typography.bodySmall,
                color = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.6f)
            )
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Pairing code input
        DeepNetCard(
            modifier = Modifier.fillMaxWidth(),
            variant = DeepNetCardVariant.NODE,
            enableGlow = true
        ) {
            Text(
                text = "Pairing Code",
                style = MaterialTheme.typography.labelMedium,
                color = DeepNetColors.OnSurfaceVariant
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Enter the code shown on your PC",
                style = MaterialTheme.typography.bodySmall,
                color = DeepNetColors.OnSurfaceVariant.copy(alpha = 0.7f)
            )
            Spacer(modifier = Modifier.height(12.dp))
            OutlinedTextField(
                value = pairingCode,
                onValueChange = { pairingCode = it.uppercase() },
                placeholder = { Text("QD-7X3K", fontFamily = FontFamily.Monospace) },
                modifier = Modifier.fillMaxWidth(),
                enabled = !isPairing,
                textStyle = LocalTextStyle.current.copy(
                    fontFamily = FontFamily.Monospace,
                    fontSize = 24.sp,
                    fontWeight = FontWeight.Bold,
                    textAlign = TextAlign.Center,
                    letterSpacing = 4.sp
                ),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = DeepNetColors.Primary,
                    unfocusedBorderColor = DeepNetColors.Primary.copy(alpha = 0.5f),
                    cursorColor = DeepNetColors.Primary,
                    focusedTextColor = DeepNetColors.Primary,
                    unfocusedTextColor = DeepNetColors.Primary
                ),
                singleLine = true
            )
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Error display
        error?.let {
            DeepNetCard(
                modifier = Modifier.fillMaxWidth(),
                variant = DeepNetCardVariant.ALERT
            ) {
                Text(
                    text = it,
                    color = DeepNetColors.Error,
                    fontSize = 13.sp
                )
            }
            Spacer(modifier = Modifier.height(16.dp))
        }

        // Pair button
        if (isPairing) {
            DeepNetLoadingIndicator(text = "PAIRING...")
        } else {
            DeepNetButton(
                onClick = {
                    if (pairingCode.isNotBlank()) {
                        onServerUrlChange(editableUrl)
                        onPair(editableUrl, pairingCode.trim())
                    }
                },
                enabled = pairingCode.isNotBlank(),
                variant = DeepNetButtonVariant.PRIMARY,
                icon = Icons.Default.Link,
                text = "PAIR DEVICE",
                modifier = Modifier.fillMaxWidth(0.7f)
            )
        }

        Spacer(modifier = Modifier.weight(1f))

        // Footer
        Text(
            text = "Generate a code on your PC:",
            fontSize = 12.sp,
            color = DeepNetColors.OnSurfaceVariant,
            textAlign = TextAlign.Center
        )
        Spacer(modifier = Modifier.height(4.dp))
        DeepNetCard(
            modifier = Modifier.fillMaxWidth(),
            variant = DeepNetCardVariant.TERMINAL
        ) {
            Text(
                text = "curl -X POST http://localhost:8080/api/pair/generate \\\n  -H 'Content-Type: application/json' \\\n  -d '{\"h_id\": \"human-yourname\"}'",
                fontFamily = FontFamily.Monospace,
                fontSize = 10.sp,
                color = DeepNetColors.Primary,
                lineHeight = 14.sp
            )
        }
    }
}
