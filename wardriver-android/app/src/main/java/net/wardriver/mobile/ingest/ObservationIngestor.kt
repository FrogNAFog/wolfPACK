package net.wardriver.mobile.ingest

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.db.NetworkSummaryEntity
import net.wardriver.mobile.db.SessionMapMarkerEntity
import net.wardriver.mobile.db.ObservationEntity
import net.wardriver.mobile.db.ProbeObservationEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.json.JSONArray
import org.json.JSONObject

/**
 * PackLink observation ingest: unpack → enrichment gates → SQLite append.
 *
 * Gates (Rust `enrichment.rs`): trusted clock, GPS usable (one fresh live fix on new wardrive
 * cold start only), outside home zone suppression, scan not held, denylist, GPS anchor match,
 * MacTable 50 m revisit dedup.
 *
 * MacTable + next_id live in a Rust enrichment session for this run only
 * (see `enrichment_session.rs`); not seeded from Room.
 */
class ObservationIngestor(
    private val db: WardriverDatabase,
    private val sessionId: Long,
    private val requireInitialFreshGps: Boolean = false,
) {
    // Dedicated ingest worker: serialized bulks, SupervisorJob isolates failures from wardrive scope.
    private val ingestDispatcher = Dispatchers.IO.limitedParallelism(1)
    private val ingestScope = CoroutineScope(SupervisorJob() + ingestDispatcher)

    private var paused = false
    private var rustSessionOpen = false
    private val gpsLock = Any()
    private val gpsTimeline = GpsTimeline()
    private var initialFreshGpsSatisfied = !requireInitialFreshGps
    private var gpsUsable = initialFreshGpsSatisfied
    private var insideHomeGeofence = false
    private var denylistBssids = emptyList<String>()
    private var denylistSsids = emptyList<String>()
    private var pendingFrameCount = 0
    private var replayGpsHydrated = false

    fun setPaused(p: Boolean) {
        paused = p
    }

    fun setDenylists(bssids: List<String>, ssids: List<String>) {
        denylistBssids = bssids
        denylistSsids = ssids
    }

    fun setInsideHomeGeofence(inside: Boolean) {
        insideHomeGeofence = inside
    }

    /** True when CSV enqueue gate is open (fresh GPS latched, or skipped on resume/catch-up). */
    fun isCsvGateOpen(): Boolean = gpsUsable

    /** Open an empty Rust enrichment session (per-run MacTable / CellTable). */
    suspend fun openSession() {
        if (rustSessionOpen) return
        if (!NativeCore.openEnrichmentSession(sessionId)) {
            throw IllegalStateException("enrichment session open failed for session $sessionId")
        }
        rustSessionOpen = true
    }

    /** Cancel ingest worker and close Rust enrichment session. */
    fun shutdown() {
        ingestScope.cancel()
        synchronized(gpsLock) {
            initialFreshGpsSatisfied = !requireInitialFreshGps
            gpsUsable = initialFreshGpsSatisfied
        }
        gpsTimeline.close()
        if (rustSessionOpen) {
            NativeCore.closeEnrichmentSession(sessionId)
            rustSessionOpen = false
        }
    }

    /**
     * Ingest phone-sourced cell towers at GPS fix cadence (throttled by caller).
     * Applies the same CSV gates as gateway observations (no BSSID/SSID denylists).
     */
    suspend fun ingestCellTowers(
        cells: List<CellTowerCollector.CellObservation>,
        observedUnixMs: Long,
        lat: Double,
        lon: Double,
        accuracyM: Float,
        altitudeM: Double?,
    ) {
        if (paused || cells.isEmpty()) return
        ensureRustSessionOpen()
        val gpsSnapshot =
            synchronized(gpsLock) {
                gpsTimeline.prune(ingestNowMs())
                gpsUsable to gpsTimeline.toJsonArray()
            }
        val cellsJson = JSONArray()
        for (cell in cells) {
            cellsJson.put(
                CellTowerCollector.cellToJson(
                    cell = cell,
                    observedUnixMs = observedUnixMs,
                    lat = lat,
                    lon = lon,
                    altitudeM = altitudeM?.toInt(),
                    accuracyM = accuracyM,
                ),
            )
        }
        val req =
            JSONObject().apply {
                put("session_id", sessionId)
                put("trusted_clock", trustedClock())
                put("gps_usable", gpsSnapshot.first)
                put("inside_home_geofence", insideHomeGeofence)
                put("scan_held", false)
                put("fast_mode", false)
                put("gps_hist", gpsSnapshot.second)
                put("cells", cellsJson)
            }
        val result = NativeCore.ingestCells(req.toString())
        if (result == null) {
            return
        }
        for (obs in result.stored) {
            storeObservation(obs)
            ObservationIngestStats.recordApStored()
        }
        for (reason in result.dropped) {
            ObservationIngestStats.recordApDropped(reason)
        }
    }

    /** Append one GPS fix to the enrichment timeline using the fix timestamp. */
    fun onGpsFix(
        unixMs: Long,
        lat: Double,
        lon: Double,
        accuracyM: Float,
        altitudeM: Double? = null,
        isFresh: Boolean = true,
    ) {
        synchronized(gpsLock) {
            val fixUsable = gpsTimeline.isUsable(accuracyM) && GpsTimeline.isValidFixTime(unixMs)
            if (!initialFreshGpsSatisfied && requireInitialFreshGps) {
                if (fixUsable && isFresh) {
                    initialFreshGpsSatisfied = true
                }
            }
            gpsUsable = initialFreshGpsSatisfied
            if (fixUsable) {
                gpsTimeline.append(unixMs, lat, lon, accuracyM, altitudeM)
            }
        }
    }

    fun onPackLinkFrame(frame: ByteArray) {
        if (paused || frame.size < 4) return
        val unpacked = NativeCore.unpackPackLinkFrame(frame)
        if (unpacked is NativeCore.PackLinkFrameResult.Error) return
        val ok = unpacked as NativeCore.PackLinkFrameResult.Ok
        when (ok.msgType) {
            MSG_OBSERVATION_BULK ->
                ingestScope.launch {
                    try {
                        ingestObservationBulk(frame.copyOf(), failOnInvalid = false)
                    } catch (e: Exception) {
                        ObservationIngestStats.recordBulkIngestFailed(e)
                        Log.e(TAG, "live observation bulk ingest failed", e)
                    }
                }
            MSG_PROBE_OBSERVATION_BULK ->
                ingestScope.launch {
                    try {
                        ingestProbeObservationBulk(frame.copyOf())
                    } catch (e: Exception) {
                        ObservationIngestStats.recordBulkIngestFailed(e)
                        Log.e(TAG, "live probe bulk ingest failed", e)
                    }
                }
        }
    }

    /**
     * Replay ingest stays on the caller's coroutine so acknowledgement cannot overtake Room writes.
     */
    suspend fun onReplayPackLinkFrame(
        frame: ByteArray,
        failOnInvalid: Boolean = true,
    ) {
        check(!paused) { "observation replay paused" }
        require(frame.size >= 4) { "truncated replay frame" }
        val unpacked = NativeCore.unpackPackLinkFrame(frame)
        require(unpacked is NativeCore.PackLinkFrameResult.Ok) { "invalid replay frame" }
        ensureReplayGpsHydrated()
        if (unpacked.msgType == MSG_OBSERVATION_BULK) {
            withContext(ingestDispatcher) {
                ingestObservationBulk(frame, failOnInvalid = failOnInvalid)
            }
        }
        if (unpacked.msgType == MSG_PROBE_OBSERVATION_BULK) {
            withContext(ingestDispatcher) {
                ingestProbeObservationBulk(frame)
            }
        }
    }

    private suspend fun ensureReplayGpsHydrated() {
        if (replayGpsHydrated) return
        val now = ingestNowMs()
        val minUnix = now - GpsTimeline.GPS_HIST_RETENTION_MS
        try {
            val points = db.routeDao().betweenUnixMs(minUnix, now)
            synchronized(gpsLock) {
                gpsTimeline.mergeRoutePoints(points, now)
                // Route hydration prepares anchor matching only; CSV gate stays on live fresh GPS.
            }
        } catch (e: Exception) {
            throw IllegalStateException("replay GPS hydration failed", e)
        }
        replayGpsHydrated = true
    }

    private suspend fun ensureRustSessionOpen() {
        if (!rustSessionOpen) {
            openSession()
        }
    }

    private suspend fun ingestObservationBulk(
        frame: ByteArray,
        failOnInvalid: Boolean = false,
    ) {
        pendingFrameCount++
        try {
            ensureRustSessionOpen()
            ObservationIngestStats.recordBulkReceived()
            val nowMs = ingestNowMs()
            val fastMode = pendingFrameCount > FAST_MODE_QUEUE_THRESHOLD
            val gpsSnapshot =
                synchronized(gpsLock) {
                    gpsTimeline.prune(nowMs)
                    gpsUsable to gpsTimeline.toJsonArray()
                }
            val ctx =
                JSONObject().apply {
                    put("session_id", sessionId)
                    put("trusted_clock", trustedClock())
                    put("gps_usable", gpsSnapshot.first)
                    put("inside_home_geofence", insideHomeGeofence)
                    put("scan_held", false)
                    put("fast_mode", fastMode)
                    put("gps_hist", gpsSnapshot.second)
                    put("denylist_bssids", JSONArray(denylistBssids))
                    put("denylist_ssids", JSONArray(denylistSsids))
                    put("fail_on_invalid", failOnInvalid)
                    put("now_ms", nowMs)
                }
            val result = NativeCore.ingestObservationBulk(frame, ctx.toString())
            if (result == null) {
                ObservationIngestStats.recordBulkUnpackFailed()
                if (failOnInvalid) {
                    throw IllegalArgumentException("invalid observation replay payload")
                }
                return
            }
            if (result.stored.isEmpty() && result.dropped.isEmpty()) {
                ObservationIngestStats.recordBulkEmpty()
                return
            }
            for (obs in result.stored) {
                storeObservation(obs)
                ObservationIngestStats.recordApStored()
            }
            for (reason in result.dropped) {
                ObservationIngestStats.recordApDropped(reason)
            }
        } finally {
            pendingFrameCount = (pendingFrameCount - 1).coerceAtLeast(0)
        }
    }

    private suspend fun ingestProbeObservationBulk(frame: ByteArray) {
        ensureRustSessionOpen()
        val resultJsonStr = NativeCore.probeObservationBulkJson(frame, sessionId)
        val resultJson = JSONObject(resultJsonStr)
        if (!resultJson.optBoolean("ok", false)) {
            ObservationIngestStats.recordBulkUnpackFailed()
            return
        }
        val obsArr = resultJson.optJSONArray("observations") ?: JSONArray()
        if (obsArr.length() == 0) {
            return
        }
        val probes = mutableListOf<ProbeObservationEntity>()
        for (i in 0 until obsArr.length()) {
            val obs = obsArr.getJSONObject(i)
            probes.add(
                ProbeObservationEntity(
                    sessionId = sessionId,
                    snifferId = obs.optInt("sniffer_id"),
                    observedUnixMs = obs.optLong("observed_unix_ms"),
                    lat = obs.optDouble("lat"),
                    lon = obs.optDouble("lon"),
                    altitudeM = obs.optInt("altitude_m").takeIf { obs.has("altitude_m") },
                    accuracyM = obs.optDouble("accuracy_m").toFloat().takeIf { obs.has("accuracy_m") },
                    srcMac = obs.optString("src_mac"),
                    ssid = obs.optString("ssid"),
                    channel = obs.optInt("channel"),
                    rssi = obs.optInt("rssi"),
                )
            )
        }
        if (probes.isNotEmpty()) {
            db.probeObservationDao().insertAll(probes)
            ObservationIngestStats.recordProbeStored(probes.size)
        }
    }

    private suspend fun storeObservation(obsJson: JSONObject) {
        val entity =
            ObservationEntity(
                sessionId = sessionId,
                snifferId = obsJson.optInt("sniffer_id"),
                observedUnixMs = obsJson.optLong("observed_unix_ms"),
                lat = obsJson.optDouble("lat"),
                lon = obsJson.optDouble("lon"),
                altitudeM = obsJson.optInt("altitude_m").takeIf { obsJson.has("altitude_m") },
                accuracyM = obsJson.optDouble("accuracy_m").toFloat().takeIf { obsJson.has("accuracy_m") },
                signalKind = obsJson.optString("signal_kind", "WIFI"),
                bssid = obsJson.optString("bssid"),
                ssid = obsJson.optString("ssid"),
                channel = obsJson.optInt("channel"),
                frequencyMhz = obsJson.optInt("frequency_mhz"),
                rssi = obsJson.optInt("rssi"),
                authMode = obsJson.optString("auth_mode"),
                rcois = obsJson.optString("rcois", ""),
                bleCapabilities = obsJson.optString("ble_capabilities", ""),
                uploaded = false,
            )
        db.observationDao().insert(entity)
        db.sessionMapMarkerDao().insert(
            SessionMapMarkerEntity(
                sessionId = entity.sessionId,
                bssid = entity.bssid,
                lat = entity.lat,
                lon = entity.lon,
                ssid = entity.ssid,
                rssi = entity.rssi,
                signalKind = entity.signalKind,
                observedUnixMs = entity.observedUnixMs,
            ),
        )
        val count = db.observationDao().countForBssid(entity.bssid)
        val summary =
            NetworkSummaryEntity(
                bssid = entity.bssid,
                ssid = entity.ssid,
                firstLat = entity.lat,
                firstLon = entity.lon,
                firstSeenUnixMs = entity.observedUnixMs,
                lastRssi = entity.rssi,
                lastLat = entity.lat,
                lastLon = entity.lon,
                lastSeenUnixMs = entity.observedUnixMs,
                observationCount = count,
                lastSignalKind = entity.signalKind,
            )
        val inserted = db.networkSummaryDao().insertIgnore(summary)
        if (inserted == -1L) {
            db.networkSummaryDao().updateLastOnly(
                bssid = entity.bssid,
                ssid = entity.ssid,
                lastRssi = entity.rssi,
                lastLat = entity.lat,
                lastLon = entity.lon,
                lastSeenUnixMs = entity.observedUnixMs,
                observationCount = count,
                lastSignalKind = entity.signalKind,
            )
        }
    }

    private fun ingestNowMs(): Long = WardriveClock.nowUnixMs()

    private fun trustedClock(): Boolean = WardriveClock.isTrusted()

    companion object {
        private const val TAG = "ObservationIngestor"

        /** PackLink `MsgType::ObservationBulk` */
        private const val MSG_OBSERVATION_BULK = 5

        /** PackLink `MsgType::ProbeObservationBulk` */
        private const val MSG_PROBE_OBSERVATION_BULK = 20

        /** WiGLE route logging: skip fixes worse than this (m). */
        const val ROUTE_MAX_ACCURACY_M = 25f

        /** Replay bulk older than control retention — distinct from live `no_gps_match`. */
        const val STALE_REPLAY_REASON = "stale_replay"

        /** When ingest queue depth exceeds this, enable fast_mode dedup peek. */
        private const val FAST_MODE_QUEUE_THRESHOLD = 8
    }
}
