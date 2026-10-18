package net.wardriver.mobile.ui.sniffers

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.FleetRepository
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackSectionLabel
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun NearbySniffersSection(
    pending: List<FleetRepository.SnifferStatusRow>,
    connection: GatewayWifiRepository.ConnectionState,
    gatewayNowMs: Long,
    onAdopt: (snifferId: Int, mac: String) -> Unit,
    onRetryConnect: () -> Unit,
    onOpenGatewaySettings: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val connected = connection is GatewayWifiRepository.ConnectionState.Connected

    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        PackSectionLabel("Nearby sniffers")
        Text(
            text =
                "Sniffers must be in ESP-NOW range of the gateway, not the phone. " +
                    "Status updates about every 2s when the gateway link is up.",
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
        )

        when {
            !connected -> {
                Text(
                    text = gatewayBannerText(connection),
                    style = MaterialTheme.typography.bodyMedium,
                    color = PackColor.Muted,
                    modifier = Modifier.padding(top = 4.dp),
                )
                RowActions(
                    onRetryConnect = onRetryConnect,
                    onOpenGatewaySettings = onOpenGatewaySettings,
                )
            }
            pending.isEmpty() -> {
                Text(
                    text = "Power sniffers near the gateway; they appear within about 2s.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = PackColor.Muted,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
            else -> {
                pending.forEach { row ->
                    NearbySnifferCard(
                        row = row,
                        gatewayNowMs = gatewayNowMs,
                        gatewayConnected = connected,
                        onAdopt = { onAdopt(row.snifferId, row.mac) },
                    )
                }
            }
        }
    }
}

@Composable
private fun RowActions(
    onRetryConnect: () -> Unit,
    onOpenGatewaySettings: (() -> Unit)?,
) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        PackOutlinedButton(
            text = "Retry gateway connect",
            onClick = onRetryConnect,
            modifier = Modifier.fillMaxWidth(),
        )
        onOpenGatewaySettings?.let { openSettings ->
            PackOutlinedButton(
                text = "Gateway settings",
                onClick = openSettings,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

private fun gatewayBannerText(connection: GatewayWifiRepository.ConnectionState): String =
    when (connection) {
        GatewayWifiRepository.ConnectionState.Connecting ->
            "Connecting to gateway..."
        GatewayWifiRepository.ConnectionState.Reconnecting ->
            "Reconnecting to gateway..."
        is GatewayWifiRepository.ConnectionState.Error ->
            "Gateway error: ${connection.message}"
        GatewayWifiRepository.ConnectionState.Disconnected ->
            "Gateway not connected. Adopt and connect a wolfPACK gateway to discover sniffers."
        GatewayWifiRepository.ConnectionState.Connected -> ""
    }
