package net.wardriver.mobile.ui.sniffers

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.FleetRepository
import net.wardriver.mobile.db.FleetRosterEntity
import net.wardriver.mobile.ui.components.PackSectionLabel
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun AdoptingSniffersSection(
    pending: List<FleetRosterEntity>,
    liveStatusById: Map<Int, FleetRepository.SnifferStatusRow>,
    gatewayNowMs: Long,
    gatewayConnected: Boolean,
    policyAckOk: Boolean,
    adoptFailedIds: Set<Int>,
    onCancel: (Int) -> Unit,
    onRetry: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (pending.isEmpty()) return

    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        PackSectionLabel("Adopting")
        Text(
            text = "Gateway retries ControlWelcome until SnifferAck confirms bind (about 60 s).",
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
        )
        pending.forEach { row ->
            val live = liveStatusById[row.snifferId]
            val subStatus =
                live?.adoptStatusLabel(gatewayConnected, policyAckOk)
                    ?: when {
                        !gatewayConnected -> "Waiting for gateway connect"
                        !policyAckOk -> "Waiting for gateway policy ack"
                        else -> "Waiting for gateway confirmation"
                    }
            val failed = row.snifferId in adoptFailedIds || live?.adoptRetryable() == true
            AdoptingSnifferCard(
                row = row,
                liveStatus = live,
                gatewayNowMs = gatewayNowMs,
                gatewayConnected = gatewayConnected,
                subStatus = subStatus,
                failed = failed,
                onCancel = { onCancel(row.snifferId) },
                onRetry = { onRetry(row.snifferId) },
            )
        }
    }
}
