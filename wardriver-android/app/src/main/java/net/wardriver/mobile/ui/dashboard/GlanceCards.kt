package net.wardriver.mobile.ui.dashboard

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.data.StatsRepository
import net.wardriver.mobile.db.DetectionEntity
import net.wardriver.mobile.ui.components.PackCard
import net.wardriver.mobile.ui.components.PackGlanceCard
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.components.PackSectionLabel
import net.wardriver.mobile.ui.theme.PackColor

/** Mirror of control web dashboard glance grid (`web_dashboard_body.html`). */
@Composable
fun DashboardGlanceGrid(
    stats: StatsRepository.DashboardStats,
    modifier: Modifier = Modifier,
) {
    val tracker = remember { IncrementStalenessTracker() }
    var tickMs by remember { mutableLongStateOf(System.currentTimeMillis()) }

    LaunchedEffect(stats.sessionActive) {
        if (!stats.sessionActive) {
            tracker.reset()
            tickMs = System.currentTimeMillis()
            return@LaunchedEffect
        }
        while (isActive) {
            tickMs = System.currentTimeMillis()
            delay(250L)
        }
    }

    val sessionActive = stats.sessionActive
    val staleColors =
        remember(
            sessionActive,
            stats.observationRowCount,
            stats.uniqueWifiMacs,
            stats.uniqueBleDevices,
            stats.uniqueCellTowers,
            tickMs,
        ) {
            if (!sessionActive) {
                StaleGlanceColors(
                    observationRows = PackColor.Text,
                    uniqueWifi = PackColor.Text,
                    uniqueBle = PackColor.Text,
                    cellTowers = PackColor.Text,
                )
            } else {
                val now = tickMs
                tracker.noteSample(
                    StalenessMetric.ObservationRows,
                    stats.observationRowCount.toLong(),
                    now,
                )
                tracker.noteSample(
                    StalenessMetric.UniqueWifiMacs,
                    stats.uniqueWifiMacs.toLong(),
                    now,
                )
                tracker.noteSample(
                    StalenessMetric.UniqueBleDevices,
                    stats.uniqueBleDevices.toLong(),
                    now,
                )
                tracker.noteSample(
                    StalenessMetric.CellTowers,
                    stats.uniqueCellTowers.toLong(),
                    now,
                )
                StaleGlanceColors(
                    observationRows =
                        tracker.colorForMetric(StalenessMetric.ObservationRows, now)
                            ?: PackColor.Good,
                    uniqueWifi =
                        tracker.colorForMetric(StalenessMetric.UniqueWifiMacs, now)
                            ?: PackColor.Good,
                    uniqueBle =
                        tracker.colorForMetric(StalenessMetric.UniqueBleDevices, now)
                            ?: PackColor.Good,
                    cellTowers =
                        tracker.colorForMetric(StalenessMetric.CellTowers, now)
                            ?: PackColor.Good,
                )
            }
        }

    fun staleColor(metric: StalenessMetric): Color =
        when (metric) {
            StalenessMetric.ObservationRows -> staleColors.observationRows
            StalenessMetric.UniqueWifiMacs -> staleColors.uniqueWifi
            StalenessMetric.UniqueBleDevices -> staleColors.uniqueBle
            StalenessMetric.CellTowers -> staleColors.cellTowers
        }

    val cards = buildGlanceCards(stats, staleColor = ::staleColor)
    Column(
        modifier = modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        cards.chunked(2).forEach { row ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                row.forEach { card ->
                    PackGlanceCard(
                        label = card.label,
                        value = card.value,
                        foot = card.foot,
                        valueColor = card.valueColor,
                        footColor = card.footColor,
                        valueAnnotated = card.valueAnnotated,
                        modifier = Modifier.weight(1f),
                    )
                }
                if (row.size == 1) {
                    Spacer(modifier = Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
fun WolfpackHealthCard(
    stats: StatsRepository.DashboardStats,
    modifier: Modifier = Modifier,
) {
    val info = stats.gatewayInfo ?: return
    val waiters = stats.peerJoinWaiters ?: 0
    val slotsFree = stats.peerSlotsFree
    val capPressure = waiters > 0 && slotsFree == 0
    val peersActive = stats.espNowPeersActive ?: 0

    PackCard(
        modifier = modifier.padding(horizontal = 12.dp, vertical = 4.dp),
    ) {
        PackSectionLabel("wolfPACK")
        Text(
            text = "ESP-NOW peers: $peersActive / ${stats.espNowPeerCap}",
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.padding(top = 4.dp),
        )
        Text(
            text = "Join queue: $waiters",
            style = MaterialTheme.typography.bodyMedium,
        )
        val spoolPct = stats.spoolUsedPct
        if (spoolPct != null) {
            Text(
                text = "Spool: $spoolPct% used",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        val foot =
            when {
                slotsFree != null && capPressure ->
                    "$slotsFree slots free — queue blocked at peer cap"
                slotsFree != null -> "$slotsFree slots free"
                capPressure -> "Queue blocked at peer cap"
                else -> ""
            }
        if (foot.isNotEmpty()) {
            Text(
                text = foot,
                style = MaterialTheme.typography.bodySmall,
                color = if (capPressure) PackColor.Err else PackColor.Text,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        Text(
            text = "Gateway ${info.gatewayMac}",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

@Composable
fun GatewayWifiCard(
    connectionState: GatewayWifiRepository.ConnectionState,
    hasTrustedGateway: Boolean,
    onAdopt: () -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onManage: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val (label, tint) = wifiStatePresentation(connectionState)
    val connecting =
        connectionState is GatewayWifiRepository.ConnectionState.Connecting ||
            connectionState is GatewayWifiRepository.ConnectionState.Reconnecting
    PackCard(
        modifier = modifier.padding(horizontal = 12.dp, vertical = 4.dp),
    ) {
        PackSectionLabel("Gateway")
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            color = tint,
            modifier = Modifier.padding(top = 4.dp),
        )
        when (connectionState) {
            is GatewayWifiRepository.ConnectionState.Connected -> {
                Row(modifier = Modifier.padding(top = 8.dp)) {
                    PackOutlinedButton(
                        text = "Disconnect",
                        onClick = onDisconnect,
                        enabled = !connecting,
                        modifier = Modifier.padding(end = 8.dp),
                    )
                    PackPrimaryButton(text = "Manage", onClick = onManage)
                }
            }
            is GatewayWifiRepository.ConnectionState.Error -> {
                PackOutlinedButton(
                    text = "Adopt gateway",
                    onClick = onAdopt,
                    enabled = !connecting,
                    modifier = Modifier.padding(top = 8.dp),
                )
            }
            GatewayWifiRepository.ConnectionState.Connecting -> {
                PackOutlinedButton(
                    text = "Connecting…",
                    onClick = {},
                    enabled = false,
                    modifier = Modifier.padding(top = 8.dp),
                )
            }
            GatewayWifiRepository.ConnectionState.Reconnecting -> {
                PackOutlinedButton(
                    text = "Reconnecting…",
                    onClick = {},
                    enabled = false,
                    modifier = Modifier.padding(top = 8.dp),
                )
            }
            GatewayWifiRepository.ConnectionState.Disconnected -> {
                if (hasTrustedGateway) {
                    PackOutlinedButton(
                        text = "Connect",
                        onClick = onConnect,
                        enabled = !connecting,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                } else {
                    PackOutlinedButton(
                        text = "Connect gateway",
                        onClick = onAdopt,
                        enabled = !connecting,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            }
        }
    }
}

@Composable
fun LastAlertCard(
    detection: DetectionEntity?,
    modifier: Modifier = Modifier,
) {
    PackCard(
        modifier = modifier.padding(horizontal = 12.dp, vertical = 4.dp),
    ) {
        PackSectionLabel("Last alert")
        if (detection == null) {
            Text(
                text = "No detections yet",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(top = 4.dp),
            )
        } else {
            Text(
                text = "Type: ${detection.kind}",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(top = 4.dp),
            )
            Text(
                text = detection.detail,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = lastSeenAgo(System.currentTimeMillis(), detection.unixMs),
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}

private data class StaleGlanceColors(
    val observationRows: Color,
    val uniqueWifi: Color,
    val uniqueBle: Color,
    val cellTowers: Color,
)

private data class GlanceCardModel(
    val label: String,
    val value: String,
    val foot: String? = null,
    val valueColor: Color = PackColor.Text,
    val footColor: Color = PackColor.Text,
    val valueAnnotated: AnnotatedString? = null,
)

private fun buildGlanceCards(
    stats: StatsRepository.DashboardStats,
    staleColor: (StalenessMetric) -> Color,
): List<GlanceCardModel> {
    val gatewayConnected =
        stats.gatewayWifiState is GatewayWifiRepository.ConnectionState.Connected
    val sniffersFoot =
        when {
            stats.sniffersAdopted == 0 ->
                "Tap Sniffers to adopt nearby devices"
            stats.espNowPeersActive != null ->
                "ESP-NOW ${stats.espNowPeersActive} / ${stats.espNowPeerCap}"
            else -> null
        }
    val (sniffersValue, sniffersValueColor, sniffersAnnotated) =
        sniffersGlancePresentation(stats, gatewayConnected)

    fun okColor(ok: Boolean): Color = if (ok) PackColor.Good else PackColor.Err

    return listOf(
        GlanceCardModel(
            label = "Sniffers",
            value = sniffersValue,
            foot = sniffersFoot,
            valueColor = sniffersValueColor,
            valueAnnotated = sniffersAnnotated,
        ),
        GlanceCardModel(
            label = "GPS fix",
            value = if (stats.gpsFixOk) "Yes" else "No",
            foot = stats.gpsDetail.takeIf { it.isNotBlank() },
            valueColor = okColor(stats.gpsFixOk),
        ),
        GlanceCardModel(
            label = "Observation rows",
            value = stats.observationRowCount.toString(),
            foot =
                if (stats.sessionActive) {
                    stats.observationIngestDetail
                } else {
                    null
                },
            valueColor = staleColor(StalenessMetric.ObservationRows),
        ),
        GlanceCardModel(
            label = "Unique MACs",
            value = "WiFi ${stats.uniqueWifiMacs}",
            foot = "BLE ${stats.uniqueBleDevices}",
            valueColor = staleColor(StalenessMetric.UniqueWifiMacs),
            footColor = staleColor(StalenessMetric.UniqueBleDevices),
        ),
        GlanceCardModel(
            label = "CSV gate",
            value = if (stats.csvGateOpen) "Open" else "Closed",
            foot = stats.csvGateDetail,
            valueColor = okColor(stats.csvGateOpen),
        ),
        GlanceCardModel(
            label = "Cell towers",
            value = "Cell ${stats.uniqueCellTowers}",
            foot = "from phone",
            valueColor = staleColor(StalenessMetric.CellTowers),
        ),
        GlanceCardModel(
            label = "Fleet",
            value = stats.fleetMode,
            foot = stats.fleetModeDetail,
        ),
    )
}

private fun sniffersGlancePresentation(
    stats: StatsRepository.DashboardStats,
    gatewayConnected: Boolean,
): Triple<String, Color, AnnotatedString?> {
    if (stats.sniffersAdopted == 0) {
        return Triple("0", PackColor.Text, null)
    }
    if (!gatewayConnected) {
        return Triple(stats.sniffersAdopted.toString(), PackColor.Text, null)
    }
    val live = stats.sniffersLive
    val dead = stats.sniffersDead
    if (dead <= 0) {
        return Triple("$live live", PackColor.Good, null)
    }
    val annotated =
        buildAnnotatedString {
            withStyle(SpanStyle(color = PackColor.Good)) {
                append("$live live")
            }
            withStyle(SpanStyle(color = PackColor.Text)) {
                append(" · ")
            }
            withStyle(SpanStyle(color = PackColor.Err)) {
                append("$dead dead")
            }
        }
    return Triple("$live live · $dead dead", PackColor.Text, annotated)
}

@Composable
private fun wifiStatePresentation(state: GatewayWifiRepository.ConnectionState): Pair<String, Color> {
    return when (state) {
        GatewayWifiRepository.ConnectionState.Connected ->
            "Connected" to PackColor.Good
        GatewayWifiRepository.ConnectionState.Connecting ->
            "Connecting…" to PackColor.Sun
        GatewayWifiRepository.ConnectionState.Reconnecting ->
            "Reconnecting…" to PackColor.Sun
        GatewayWifiRepository.ConnectionState.Disconnected ->
            "Disconnected" to PackColor.Muted
        is GatewayWifiRepository.ConnectionState.Error ->
            state.message to PackColor.Err
    }
}

/** Relative time label matching web dashboard `lastSeenAgo`. */
fun lastSeenAgo(nowMs: Long, lastMs: Long): String {
    if (lastMs <= 0L) return "-"
    val d = nowMs - lastMs
    if (d < 0 || d > 86_400_000L) return "-"
    if (d < 1500) return "just now"
    val sec = d / 1000
    if (sec < 60) return "${sec}s ago"
    val min = sec / 60
    if (min < 120) return "${min}m ago"
    return "${min / 60}h ago"
}
