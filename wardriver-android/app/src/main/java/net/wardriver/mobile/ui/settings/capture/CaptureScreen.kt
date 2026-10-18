package net.wardriver.mobile.ui.settings.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.wardriverApp

private const val CAPTURE_DURATION_MS = 60_000L

/** WiFi probe capture controls (PackLink CaptureCmd / CaptureStatus over gateway TCP). */
@Composable
fun CaptureScreen(
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val gatewayWifi = context.wardriverApp().gatewayWifiRepository
    val connection by gatewayWifi.connectionState.collectAsState()
    val captureStatus by gatewayWifi.captureStatusFlow.collectAsState()

    LaunchedEffect(Unit) {
        if (connection is GatewayWifiRepository.ConnectionState.Disconnected) {
            gatewayWifi.connectPreferred()
        }
    }

    val connected = connection is GatewayWifiRepository.ConnectionState.Connected
    val active = captureStatus?.active == true
    val status = captureStatus
    val progress =
        if (active && status != null) {
            val remaining = status.remainingMs.coerceIn(0L, CAPTURE_DURATION_MS)
            1f - (remaining.toFloat() / CAPTURE_DURATION_MS.toFloat())
        } else {
            0f
        }

    Column(
        modifier = modifier.fillMaxSize().padding(16.dp),
    ) {
        Text(
            text = "WiFi probe capture",
            style = MaterialTheme.typography.headlineSmall,
        )
        Text(
            text =
                when (connection) {
                    is GatewayWifiRepository.ConnectionState.Connected -> "Gateway connected"
                    is GatewayWifiRepository.ConnectionState.Connecting -> "Connecting to gateway…"
                    is GatewayWifiRepository.ConnectionState.Reconnecting ->
                        "Reconnecting to gateway…"
                    is GatewayWifiRepository.ConnectionState.Error ->
                        "Gateway: ${(connection as GatewayWifiRepository.ConnectionState.Error).message}"
                    GatewayWifiRepository.ConnectionState.Disconnected -> "Gateway not connected"
                },
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(top = 8.dp, bottom = 16.dp),
        )

        if (active) {
            LinearProgressIndicator(
                progress = { progress.coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp),
            )
            captureStatus?.let { st ->
                Text(
                    text =
                        "Remaining ${st.remainingMs / 1000}s - " +
                            "frags ${st.fragsRx} - ${st.bytesWritten} bytes",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(bottom = 16.dp),
                )
            }
        }

        PackPrimaryButton(
            text = "Start 60s WiFi probe capture",
            onClick = { gatewayWifi.startWifiProbeCapture(durationSec = 60) },
            enabled = connected && !active,
            modifier = Modifier.fillMaxWidth(),
        )

        PackPrimaryButton(
            text = "Download PCAP (requires gateway connection)",
            onClick = { },
            enabled = false,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )

        if (!connected) {
            Text(
                text = "Adopt a wolfPACK gateway and connect over Wi-Fi Aware to start capture.",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 16.dp),
            )
        }
    }
}
