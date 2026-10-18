package net.wardriver.mobile.ui.sniffers

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
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
fun FleetRosterSection(
    roster: List<FleetRosterEntity>,
    liveStatusById: Map<Int, FleetRepository.SnifferStatusRow>,
    gatewayNowMs: Long,
    gatewayConnected: Boolean,
    manualBandMode: Boolean,
    onRemove: (Int) -> Unit,
    onSetRole: (Int, String) -> Unit,
    onSetBand: (Int, String) -> Unit,
    onSetPinnedChannel: (Int, Int?) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        PackSectionLabel("My fleet")
        if (roster.isEmpty()) {
            Text(
                text = "No adopted sniffers yet - adopt from Nearby above.",
                style = MaterialTheme.typography.bodyMedium,
                color = PackColor.Muted,
            )
        } else {
            roster.forEach { row ->
                FleetSnifferCard(
                    row = row,
                    liveStatus = liveStatusById[row.snifferId],
                    gatewayNowMs = gatewayNowMs,
                    gatewayConnected = gatewayConnected,
                    manualBandMode = manualBandMode,
                    onRemove = { onRemove(row.snifferId) },
                    onToggleRole = {
                        val next =
                            if (row.role == FleetRepository.ROLE_WIFI) {
                                FleetRepository.ROLE_BLE
                            } else {
                                FleetRepository.ROLE_WIFI
                            }
                        onSetRole(row.snifferId, next)
                    },
                    onSetBand = { band -> onSetBand(row.snifferId, band) },
                    onSetPinnedChannel = { ch -> onSetPinnedChannel(row.snifferId, ch) },
                )
            }
        }
    }
}
