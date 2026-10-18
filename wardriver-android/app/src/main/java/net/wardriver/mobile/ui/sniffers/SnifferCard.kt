package net.wardriver.mobile.ui.sniffers

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.FleetRepository
import net.wardriver.mobile.db.FleetRosterEntity
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackOutlinedTextField
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun NearbySnifferCard(
    row: FleetRepository.SnifferStatusRow,
    gatewayNowMs: Long,
    gatewayConnected: Boolean,
    onAdopt: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val stale = snifferIsQuietStale(gatewayNowMs, row.lastSeenMs, gatewayConnected)
    val bindingHint =
        if (row.adoptReason == FleetRepository.ADOPT_REASON_POSSIBLE_EXISTING_BINDING) {
            "May be bound to another control - wait 20s, power-cycle, or clear sniffer NVS"
        } else {
            null
        }
    val staleSubtitle =
        bindingHint
            ?: if (stale) {
                "Not heard recently - still in range? Adopt may need a retry"
            } else {
                null
            }
    SnifferCardShell(stale = stale, modifier = modifier) {
        SnifferCardHeader(
            snifferId = row.snifferId,
            mac = row.mac,
            stale = stale,
        )
        SnifferCardMeta(
            parts =
                listOf(
                    row.roleLabel().uppercase(),
                    "seen ${formatSnifferLastSeenAgo(gatewayNowMs, row.lastSeenMs)}",
                    "CPU ${formatPct(row.cpuLoadPct)}%",
                    "heap ${formatPct(row.heapAvailPct)}%",
                    stateLabel(row.state),
                ),
            stale = stale,
        )
        staleSubtitle?.let { subtitle ->
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Err.copy(alpha = 0.82f),
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        PackPrimaryButton(
            text = "Adopt",
            onClick = onAdopt,
            modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
        )
    }
}

@Composable
fun AdoptingSnifferCard(
    row: FleetRosterEntity,
    liveStatus: FleetRepository.SnifferStatusRow?,
    gatewayNowMs: Long,
    gatewayConnected: Boolean,
    subStatus: String,
    failed: Boolean = false,
    onCancel: () -> Unit,
    onRetry: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
) {
    val lastSeenMs = liveStatus?.lastSeenMs ?: row.lastSeenMs ?: 0L
    val stale = snifferIsQuietStale(gatewayNowMs, lastSeenMs, gatewayConnected)
    SnifferCardShell(stale = stale, modifier = modifier) {
        SnifferCardHeader(
            snifferId = row.snifferId,
            mac = row.mac,
            stale = stale,
            slotLabel = "slot ${row.fleetSlot}",
        )
        SnifferCardMeta(
            parts =
                listOf(
                    row.role.uppercase(),
                    subStatus,
                    "seen ${formatSnifferLastSeenAgo(gatewayNowMs, liveStatus?.lastSeenMs ?: row.lastSeenMs)}",
                ),
            stale = stale,
        )
        PackOutlinedButton(
            text = "Cancel",
            onClick = onCancel,
            modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
        )
        if (failed && onRetry != null) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PackOutlinedButton(
                    text = "Retry",
                    onClick = onRetry,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun FleetSnifferCard(
    row: FleetRosterEntity,
    liveStatus: FleetRepository.SnifferStatusRow?,
    gatewayNowMs: Long,
    gatewayConnected: Boolean,
    manualBandMode: Boolean,
    onRemove: () -> Unit,
    onToggleRole: () -> Unit,
    onSetBand: (String) -> Unit,
    onSetPinnedChannel: (Int?) -> Unit,
    modifier: Modifier = Modifier,
) {
    val lastSeenMs = liveStatus?.lastSeenMs ?: row.lastSeenMs ?: 0L
    val stale = snifferIsQuietStale(gatewayNowMs, lastSeenMs, gatewayConnected)
    // Prefer live gateway ACK role when connected; roster `row.role` is the operator intent.
    val displayRole =
        if (gatewayConnected && liveStatus != null) {
            liveStatus.roleLabel()
        } else {
            row.role
        }
    val isWifi = row.role == FleetRepository.ROLE_WIFI
    var pinText by rememberSaveable(row.snifferId, row.pinnedChannel) {
        mutableStateOf(row.pinnedChannel?.toString().orEmpty())
    }
    SnifferCardShell(stale = stale, modifier = modifier) {
        SnifferCardHeader(
            snifferId = row.snifferId,
            mac = row.mac,
            stale = stale,
            slotLabel = "slot ${row.fleetSlot}",
        )
        val metaParts =
            buildList {
                add(displayRole.uppercase())
                if (gatewayConnected && liveStatus != null && liveStatus.roleLabel() != row.role) {
                    add("set ${row.role.uppercase()}")
                }
                if (isWifi) {
                    add(row.band)
                    liveStatus?.assignedBandLabel()?.takeIf { it.isNotBlank() }?.let {
                        add("live $it")
                    }
                    row.pinnedChannel?.let { add("pin $it") }
                    liveStatus?.livePinnedChannel()?.let { add("live pin $it") }
                    liveStatus?.assignedChannelsPreview?.takeIf { it.isNotEmpty() }?.let { preview ->
                        add("ch ${preview.joinToString(",")}")
                    }
                }
                add("seen ${formatSnifferLastSeenAgo(gatewayNowMs, liveStatus?.lastSeenMs ?: row.lastSeenMs)}")
                add("CPU ${formatPct(liveStatus?.cpuLoadPct ?: row.cpuLoadPct)}%")
                add("heap ${formatPct(liveStatus?.heapAvailPct ?: row.heapAvailPct)}%")
                add(stateLabel(liveStatus?.state ?: row.state))
            }
        SnifferCardMeta(parts = metaParts, stale = stale)
        if (manualBandMode && isWifi) {
            Text(
                text = "Manual band",
                style = MaterialTheme.typography.labelMedium,
                color = PackColor.Muted,
                modifier = Modifier.padding(top = 8.dp),
            )
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                modifier = Modifier.padding(top = 4.dp),
            ) {
                FilterChip(
                    selected = row.band == FleetRepository.BAND_24,
                    onClick = { onSetBand(FleetRepository.BAND_24) },
                    label = { Text("2.4 GHz") },
                    colors = fleetChipColors(),
                )
                FilterChip(
                    selected = row.band == FleetRepository.BAND_5,
                    onClick = { onSetBand(FleetRepository.BAND_5) },
                    label = { Text("5 GHz") },
                    enabled = liveStatus?.fiveGhzCapable != 0,
                    colors = fleetChipColors(),
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth().padding(top = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PackOutlinedTextField(
                    value = pinText,
                    onValueChange = { pinText = it.filter { ch -> ch.isDigit() } },
                    label = { Text("Pin channel") },
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                )
                PackOutlinedButton(
                    text = "Apply pin",
                    onClick = {
                        val ch = pinText.toIntOrNull()
                        onSetPinnedChannel(ch)
                    },
                    modifier = Modifier.weight(1f),
                )
            }
            PackOutlinedButton(
                text = "Clear pin",
                onClick = {
                    pinText = ""
                    onSetPinnedChannel(null)
                },
                modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
            )
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            PackOutlinedButton(
                text = if (row.role == FleetRepository.ROLE_WIFI) "Set BLE" else "Set WiFi",
                onClick = onToggleRole,
                modifier = Modifier.weight(1f),
            )
            PackOutlinedButton(
                text = "Remove",
                onClick = onRemove,
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Composable
private fun fleetChipColors() =
    FilterChipDefaults.filterChipColors(
        containerColor = PackColor.Surface2,
        labelColor = PackColor.Text,
        selectedContainerColor = PackColor.Cyan,
        selectedLabelColor = PackColor.Background,
    )

@Composable
private fun SnifferCardShell(
    stale: Boolean,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val borderColor = if (stale) PackColor.Err.copy(alpha = 0.9f) else PackColor.BorderChromeSubtle
    val background = if (stale) PackColor.ErrWash else PackColor.Surface2
    Surface(
        modifier =
            modifier
                .fillMaxWidth()
                .border(1.dp, borderColor, RoundedCornerShape(10.dp)),
        shape = RoundedCornerShape(10.dp),
        color = background,
    ) {
        Column(modifier = Modifier.padding(12.dp), content = content)
    }
}

@Composable
private fun SnifferCardHeader(
    snifferId: Int,
    mac: String,
    stale: Boolean,
    slotLabel: String? = null,
) {
    val titleColor = if (stale) PackColor.Err else PackColor.Text
    val idLine =
        buildString {
            append(shortSnifferId(snifferId))
            append(" - ")
            append(mac)
            slotLabel?.let {
                append(" - ")
                append(it)
            }
        }
    Text(
        text = idLine,
        style =
            MaterialTheme.typography.titleSmall.copy(
                fontFamily = FontFamily.Monospace,
            ),
        color = titleColor,
    )
}

@Composable
private fun SnifferCardMeta(
    parts: List<String>,
    stale: Boolean,
) {
    Text(
        text = parts.joinToString(" - "),
        style = MaterialTheme.typography.bodySmall,
        color = if (stale) PackColor.Err.copy(alpha = 0.82f) else PackColor.Muted,
        modifier = Modifier.padding(top = 6.dp),
    )
}
