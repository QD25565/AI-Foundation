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
import com.aifoundation.app.ui.theme.FoundationColors

/**
 * Two-phase pairing screen.
 *
 * Phase 1 (pairingCode == null):
 *   User enters server URL → taps GET CODE → app calls requestPairingCode(url).
 *
 * Phase 2 (pairingCode != null):
 *   Server returned a code. App displays it and instructs the user to run:
 *     teambook mobile-pair <code>
 *   AppRoot polls pollPairingCode() every 3 s in the background.
 *   On approval the token arrives, isPaired flips, and navigation happens automatically.
 */
@Composable
fun PairingScreen(
    serverUrl:    String,
    onRequestCode: (String) -> Unit,
    pairingCode:  String?,
    isPairing:    Boolean,
    pairingError: String?,
    onClearError: () -> Unit
) {
    var editableUrl by remember(serverUrl) { mutableStateOf(serverUrl) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(FoundationColors.Background)
            .padding(start = 24.dp, end = 24.dp, top = 24.dp, bottom = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Spacer(modifier = Modifier.height(48.dp))

        // Title
        Text(
            text         = "AI-FOUNDATION",
            fontSize     = 28.sp,
            fontWeight   = FontWeight.Black,
            fontFamily   = FontFamily.Monospace,
            color        = FoundationColors.Primary,
            letterSpacing = (-1).sp
        )
        Spacer(modifier = Modifier.height(4.dp))
        Text(
            text          = "HUMAN INTERFACE",
            fontSize      = 14.sp,
            fontFamily    = FontFamily.Monospace,
            color         = FoundationColors.Secondary,
            letterSpacing = 2.sp
        )

        Spacer(modifier = Modifier.height(48.dp))

        if (pairingCode == null) {
            // ── Phase 1: Enter server URL ─────────────────────────────────────

            FoundationCard(
                modifier = Modifier.fillMaxWidth(),
                variant  = FoundationCardVariant.TERMINAL
            ) {
                Text(
                    text  = "Server URL",
                    style = MaterialTheme.typography.labelMedium,
                    color = FoundationColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(8.dp))
                OutlinedTextField(
                    value         = editableUrl,
                    onValueChange = { editableUrl = it },
                    placeholder   = { Text("http://192.168.x.x:8081") },
                    modifier      = Modifier.fillMaxWidth(),
                    enabled       = !isPairing,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor   = FoundationColors.Primary,
                        unfocusedBorderColor = FoundationColors.OnSurfaceVariant,
                        cursorColor          = FoundationColors.Primary,
                        focusedTextColor     = FoundationColors.OnSurface,
                        unfocusedTextColor   = FoundationColors.OnSurface
                    ),
                    singleLine = true
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text  = "Emulator: 10.0.2.2:8081 · Real device: your server's LAN IP",
                    style = MaterialTheme.typography.bodySmall,
                    color = FoundationColors.OnSurfaceVariant.copy(alpha = 0.6f)
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            // Error
            pairingError?.let {
                FoundationCard(modifier = Modifier.fillMaxWidth(), variant = FoundationCardVariant.ALERT) {
                    Text(text = it, color = FoundationColors.Error, fontSize = 13.sp)
                }
                Spacer(modifier = Modifier.height(16.dp))
            }

            // GET CODE button
            if (isPairing) {
                FoundationLoadingIndicator(text = "CONNECTING...")
            } else {
                FoundationButton(
                    onClick  = {
                        if (editableUrl.isNotBlank()) {
                            pairingError?.let { onClearError() }
                            onRequestCode(editableUrl.trim())
                        }
                    },
                    enabled  = editableUrl.isNotBlank(),
                    variant  = FoundationButtonVariant.PRIMARY,
                    icon     = Icons.Default.VpnKey,
                    text     = "GET PAIRING CODE",
                    modifier = Modifier.fillMaxWidth(0.8f)
                )
            }

            Spacer(modifier = Modifier.weight(1f))

            // Help footer
            FoundationCard(modifier = Modifier.fillMaxWidth(), variant = FoundationCardVariant.DATA) {
                Text(
                    text       = "Start the mobile API server on your machine:",
                    fontSize   = 12.sp,
                    color      = FoundationColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    text       = "ai-foundation-mobile-api",
                    fontFamily = FontFamily.Monospace,
                    fontSize   = 12.sp,
                    color      = FoundationColors.Primary
                )
            }

        } else {
            // ── Phase 2: Show code, wait for approval ─────────────────────────

            FoundationCard(
                modifier   = Modifier.fillMaxWidth(),
                variant    = FoundationCardVariant.NODE,
                enableGlow = true
            ) {
                Text(
                    text  = "Your Pairing Code",
                    style = MaterialTheme.typography.labelMedium,
                    color = FoundationColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    text          = pairingCode,
                    fontFamily    = FontFamily.Monospace,
                    fontSize      = 36.sp,
                    fontWeight    = FontWeight.Black,
                    color         = FoundationColors.Primary,
                    letterSpacing = 6.sp,
                    textAlign     = TextAlign.Center,
                    modifier      = Modifier.fillMaxWidth()
                )
                Spacer(modifier = Modifier.height(16.dp))
                HorizontalDivider(color = FoundationColors.GlassBorder)
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text  = "Run this command on your server:",
                    style = MaterialTheme.typography.bodySmall,
                    color = FoundationColors.OnSurfaceVariant
                )
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    text       = "teambook mobile-pair $pairingCode",
                    fontFamily = FontFamily.Monospace,
                    fontSize   = 13.sp,
                    fontWeight = FontWeight.Bold,
                    color      = FoundationColors.Primary
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            // Polling indicator
            FoundationCard(modifier = Modifier.fillMaxWidth(), variant = FoundationCardVariant.TERMINAL) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    FoundationLoadingIndicator()
                    Text(
                        text       = "Waiting for approval...",
                        fontFamily = FontFamily.Monospace,
                        fontSize   = 13.sp,
                        color      = FoundationColors.OnSurfaceVariant
                    )
                }
            }

            // Error (e.g. code expired)
            pairingError?.let {
                Spacer(modifier = Modifier.height(16.dp))
                FoundationCard(modifier = Modifier.fillMaxWidth(), variant = FoundationCardVariant.ALERT) {
                    Text(text = it, color = FoundationColors.Error, fontSize = 13.sp)
                }
            }

            Spacer(modifier = Modifier.weight(1f))

            Text(
                text       = "Code expires in 10 minutes",
                fontSize   = 12.sp,
                color      = FoundationColors.OnSurfaceVariant,
                textAlign  = TextAlign.Center
            )
        }
    }
}
