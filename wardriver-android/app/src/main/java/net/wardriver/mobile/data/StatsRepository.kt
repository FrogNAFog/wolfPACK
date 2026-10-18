package net.wardriver.mobile.data

import android.location.Location
import android.os.SystemClock
import net.wardriver.mobile.NativeCore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import net.wardriver.mobile.db.DetectionEntity
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.ingest.ObservationIngestStats
import net.wardriver.mobile.ingest.ObservationIngestor
import net.wardriver.mobile.ingest.WardriveClock
import net.wardriver.mobile.ui.sniffers.snifferIsQuietStale

/** Aggregated dashboard stats from Room, gateway_info, and DataStore prefs. */
class StatsRepository(
    private val db: WardriverDatabase,
    private val gatewayWifi: GatewayWifiRepository,
    private val appPreferences: AppPreferences,
    private val gpsLocation: () -> Location?,
    private val activeSessionId: () -> Long?,
    private val homeZoneLatchSuppressed: () -> Boolean = { false },
) {
    private val _stats = MutableStateFlow(DashboardStats())
    val stats: StateFlow<DashboardStats> = _stats.asStateFlow()

    private var pollJob: Job? = null

    /** Start adaptive polling: 1 s when gateway connected, 5 s offline. */
    fun startPolling(scope: CoroutineScope) {
        pollJob?.cancel()
        pollJob =
            scope.launch {
                while (isActive) {
                    refresh()
                    val connected =
                        gatewayWifi.connectionState.value is GatewayWifiRepository.ConnectionState.Connected
                    delay(if (connected) POLL_CONNECTED_MS else POLL_OFFLINE_MS)
                }
            }
    }

    fun stopPolling() {
        pollJob?.cancel()
        pollJob = null
    }

    /** One-shot refresh (also used by the polling loop). */
    suspend fun refresh() {
        val nowMs = System.currentTimeMillis()
        val scanAllowed = appPreferences.isScanAllowed()
        val sessionId = activeSessionId()
        val sessionActive = sessionId != null
        val prefsSnapshot = appPreferences.snapshot()
        val loc = gpsLocation()

        val observationCount =
            sessionId?.let { db.observationDao().countForSession(it) } ?: 0
        val uniqueWifi =
            sessionId?.let { db.observationDao().countDistinctWifiBssidsForSession(it) } ?: 0
        val uniqueBle =
            sessionId?.let { db.observationDao().countDistinctBleDevicesForSession(it) } ?: 0
        val uniqueCells =
            sessionId?.let { db.observationDao().countDistinctCellTowersForSession(it) } ?: 0
        val lastDetection = db.detectionDao().latest()

        val gatewayInfo = gatewayWifi.gatewayInfoFlow.value
        val connectionState = gatewayWifi.connectionState.value
        val gatewayConnected =
            connectionState is GatewayWifiRepository.ConnectionState.Connected

        val snifferCounts = snifferLiveDeadCounts(gatewayConnected)
        val espNowCap = ESPNOW_ENCRYPT_PEER_CAP
        val espNowActive =
            gatewayInfo?.let { cap -> cap.peerSlotsFree.let { free -> (espNowCap - free).coerceIn(0, espNowCap) } }

        val locationFixOk =
            loc != null &&
                loc.hasAccuracy() &&
                loc.accuracy > 0f &&
                loc.accuracy <= ObservationIngestor.ROUTE_MAX_ACCURACY_M
        // Glance GPS card: when wardrive is stopped show "No" with no accuracy foot.
        val gpsFixOk = sessionActive && locationFixOk
        val gpsDetail =
            if (sessionActive) {
                gpsDetailLine(loc, locationFixOk, sessionActive = true)
            } else {
                ""
            }

        val pointInHomeZone = isPointInHomeZone(prefsSnapshot, loc)
        val insideHome =
            CsvGateHelpers.csvInsideHome(
                sessionActive = sessionActive,
                latchSuppressed = homeZoneLatchSuppressed(),
                gpsFixOk = locationFixOk,
                pointInCircle = pointInHomeZone,
            )
        val clockOk = WardriveClock.isTrusted()
        val csvBlockers =
            CsvGateHelpers.csvGateBlockers(scanAllowed, clockOk, locationFixOk, insideHome)
        val csvOpen = csvBlockers.isEmpty()

        val fleetMode = fleetModeLabel(scanAllowed, sessionActive, csvOpen)
        val spoolPct =
            gatewayInfo?.let { info ->
                if (info.spoolCapacityBytes <= 0L) 0
                else ((info.spoolUsedBytes * 100L) / info.spoolCapacityBytes).toInt().coerceIn(0, 100)
            }

        val firmwareLine =
            gatewayInfo?.let { info ->
                "v${info.firmwareBuild} · proto ${info.protoVersion}"
            } ?: ""

        val alertBanner = buildAlertBanner(lastDetection, nowMs)
        val ingestDetail =
            if (sessionActive) {
                val packlinkBulks = gatewayWifi.observationBulkRxCount.value
                ObservationIngestStats.snapshot.value.detailLine(packlinkBulks)
            } else {
                null
            }

        _stats.value =
            DashboardStats(
                sniffersAdopted = snifferCounts.adopted,
                sniffersLive = snifferCounts.live,
                sniffersDead = snifferCounts.dead,
                sniffersActive = snifferCounts.adopted,
                sniffersMax = FLEET_SNIFFER_CAP,
                espNowPeersActive = espNowActive,
                espNowPeerCap = espNowCap,
                gpsFixOk = gpsFixOk,
                gpsDetail = gpsDetail,
                csvGateOpen = csvOpen,
                csvGateDetail = if (csvOpen) "Ready" else csvBlockers.joinToString(" · "),
                observationRowCount = observationCount,
                observationIngestDetail = ingestDetail,
                uniqueWifiMacs = uniqueWifi,
                uniqueBleDevices = uniqueBle,
                uniqueCellTowers = uniqueCells,
                fleetMode = fleetMode,
                fleetModeDetail = if (sessionActive) "Session active" else "No session",
                spoolUsedPct = spoolPct,
                peerSlotsFree = gatewayInfo?.peerSlotsFree,
                peerJoinWaiters = gatewayInfo?.peerJoinWaiters,
                gatewayWifiState = connectionState,
                gatewayInfo = gatewayInfo,
                lastDetection = lastDetection,
                alertBanner = alertBanner,
                sessionActive = sessionActive,
                scanAllowed = scanAllowed,
                firmwareLine = firmwareLine,
            )
    }

    private data class SnifferLiveDead(
        val adopted: Int,
        val live: Int,
        val dead: Int,
    )

    /** Adopted sniffers classified live vs quiet-stale when gateway is connected. */
    private suspend fun snifferLiveDeadCounts(gatewayConnected: Boolean): SnifferLiveDead {
        val adopted =
            try {
                db.fleetRosterDao().adopted()
            } catch (_: Exception) {
                emptyList()
            }
        if (adopted.isEmpty()) {
            return SnifferLiveDead(adopted = 0, live = 0, dead = 0)
        }
        if (!gatewayConnected) {
            return SnifferLiveDead(adopted = adopted.size, live = 0, dead = 0)
        }
        val gatewayNowMs = gatewayWifi.extrapolatedGatewayNowMs()
        val statusById = gatewayWifi.snifferStatusFlow.value.associateBy { it.snifferId }
        var live = 0
        var dead = 0
        for (row in adopted) {
            val lastSeenMs = statusById[row.snifferId]?.lastSeenMs ?: row.lastSeenMs ?: 0L
            if (snifferIsQuietStale(gatewayNowMs, lastSeenMs, gatewayConnected = true)) {
                dead++
            } else {
                live++
            }
        }
        return SnifferLiveDead(adopted = adopted.size, live = live, dead = dead)
    }

    private fun isPointInHomeZone(snapshot: AppPreferences.PrefsSnapshot, loc: Location?): Boolean {
        if (!snapshot.homeZoneEnabled || loc == null) return false
        val lat = snapshot.homeZoneLat ?: return false
        val lon = snapshot.homeZoneLon ?: return false
        return NativeCore.homeZoneContains(
            centerLat = lat,
            centerLon = lon,
            radiusM = snapshot.homeZoneRadiusM,
            lat = loc.latitude,
            lon = loc.longitude,
        )
    }

    private fun fleetModeLabel(scanAllowed: Boolean, sessionActive: Boolean, csvOpen: Boolean): String {
        if (!scanAllowed) return "Paused"
        if (!sessionActive) return if (scanAllowed) "Scan only" else "Idle"
        return if (csvOpen) "Wardriving" else "Scan only"
    }

    private fun gpsDetailLine(loc: Location?, fixOk: Boolean, sessionActive: Boolean): String {
        if (loc == null) {
            return when {
                fixOk -> "Fix OK"
                sessionActive -> GpsDetailHelpers.WAITING_FOR_GPS
                else -> "No fix"
            }
        }
        val fixAgeMs =
            if (loc.elapsedRealtimeNanos > 0L) {
                (SystemClock.elapsedRealtimeNanos() - loc.elapsedRealtimeNanos) / 1_000_000L
            } else {
                null
            }
        val altitude = if (loc.hasAltitude()) loc.altitude else null
        val accuracy = if (loc.hasAccuracy() && loc.accuracy > 0f) loc.accuracy else null
        val detail = GpsDetailHelpers.formatDetail(accuracy, altitude, fixAgeMs)
        return if (detail == "No fix" && fixOk) "Fix OK" else detail
    }

    private fun buildAlertBanner(detection: DetectionEntity?, nowMs: Long): AlertBanner? {
        if (detection == null) return null
        val kind = detection.kind.uppercase()
        if (
            kind.contains(AlertKind.FLOCK) ||
            kind.contains(AlertKind.DETECTION) ||
            kind.contains(AlertKind.GEOFENCE)
        ) {
            return null
        }
        val ageMs = nowMs - detection.unixMs
        if (ageMs < 0 || ageMs > ALERT_HIGHLIGHT_WINDOW_MS) return null
        val headline =
            when {
                kind.contains("SSID") -> "SSID watch alert"
                kind.contains("SURVEILLANCE") || kind.contains("AXON") -> "Mobile surveillance alert"
                else -> "Detection alert"
            }
        val subtitle =
            when {
                kind.contains("SSID") -> "SSID watch · ${detection.detail}"
                else -> detection.detail
            }
        return AlertBanner(headline = headline, subtitle = subtitle, unixMs = detection.unixMs)
    }

    data class DashboardStats(
        val sniffersAdopted: Int = 0,
        val sniffersLive: Int = 0,
        val sniffersDead: Int = 0,
        /** @deprecated Prefer [sniffersAdopted]; kept for older callers. */
        val sniffersActive: Int = 0,
        val sniffersMax: Int = FLEET_SNIFFER_CAP,
        val espNowPeersActive: Int? = null,
        val espNowPeerCap: Int = ESPNOW_ENCRYPT_PEER_CAP,
        val gpsFixOk: Boolean = false,
        val gpsDetail: String = "—",
        val csvGateOpen: Boolean = false,
        val csvGateDetail: String = "—",
        val observationRowCount: Int = 0,
        val observationIngestDetail: String? = null,
        val uniqueWifiMacs: Int = 0,
        val uniqueBleDevices: Int = 0,
        val uniqueCellTowers: Int = 0,
        val fleetMode: String = "Idle",
        val fleetModeDetail: String = "—",
        val spoolUsedPct: Int? = null,
        val peerSlotsFree: Int? = null,
        val peerJoinWaiters: Int? = null,
        val gatewayWifiState: GatewayWifiRepository.ConnectionState =
            GatewayWifiRepository.ConnectionState.Disconnected,
        val gatewayInfo: GatewayWifiRepository.GatewayInfo? = null,
        val lastDetection: DetectionEntity? = null,
        val alertBanner: AlertBanner? = null,
        val sessionActive: Boolean = false,
        val scanAllowed: Boolean = true,
        val firmwareLine: String = "",
    )

    data class AlertBanner(
        val headline: String,
        val subtitle: String,
        val unixMs: Long,
    )

    companion object {
        private const val POLL_CONNECTED_MS = 1_000L
        private const val POLL_OFFLINE_MS = 5_000L
        private const val ALERT_HIGHLIGHT_WINDOW_MS = 30_000L
        private const val FLEET_SNIFFER_CAP = 48
        private const val ESPNOW_ENCRYPT_PEER_CAP = 16
    }
}
