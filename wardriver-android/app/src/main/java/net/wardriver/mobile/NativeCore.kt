package net.wardriver.mobile

import org.json.JSONArray
import org.json.JSONObject

/**
 * JNI bridge to `wardriver-mobile-core` Rust cdylib (`libwardriver_mobile_core.so`).
 *
 * Built by Gradle task `cargoBuildMobileCore` (cargo-ndk, arm64-v8a). See docs/MOBILE_APP.md.
 */
object NativeCore {
    init {
        try {
            System.loadLibrary("wardriver_mobile_core")
        } catch (_: UnsatisfiedLinkError) {
            // Host/unit tests without JNI .so
        }
    }

    /** Revisit radius in meters (control MacTable semantics). */
    external fun revisitRadiusM(): Double

    /** PackLink frame magic (`PLNK`). */
    external fun packlinkMagic(): String

    /**
     * Unpack PackLink wire bytes; returns JSON with `msg_type`, `flags`, `seq`, `payload_hex`.
     * On failure: `{"ok":false,"error":"..."}`.
     */
    external fun unpackPackLinkFrameJson(wire: ByteArray): String

    /** Open wardrive enrichment session (empty per-run MacTable / CellTable). */
    external fun enrichmentSessionOpenJson(requestJson: String): String

    /** Close wardrive enrichment session (idempotent). */
    external fun enrichmentSessionCloseJson(requestJson: String): String

    /** Pack PolicySyncV1 from JSON; returns JSON with `wire_hex`. */
    external fun packPolicySyncJson(requestJson: String): String

    /** Pack ChannelPlanSyncV1 from JSON; returns JSON with `wire_hex`. */
    external fun packChannelPlanSyncJson(requestJson: String): String

    /** Pack CaptureCmdV1; returns JSON with `wire_hex`. */
    external fun packCaptureCmdJson(requestJson: String): String

    /** Unpack CaptureStatus PackLink frame; JSON progress fields. */
    external fun unpackCaptureStatusJson(wire: ByteArray): String

    /** Unpack SnifferStatus PackLink frame (msg type 8); JSON with `rows` array. */
    external fun unpackSnifferStatusJson(wire: ByteArray): String

    /** WiFi ingest gates + dedup (control parity, host-side). */
    external fun enrichWifiApJson(requestJson: String): String

    /** BLE ingest gates + dedup (control parity, host-side). */
    external fun enrichBleApJson(requestJson: String): String

    /** Cell tower gates + dedup (control parity, host-side). */
    external fun enrichCellJson(requestJson: String): String

    /** Unpack PolicySyncAck PackLink frame (msg type 16). */
    external fun unpackPolicySyncAckJson(wire: ByteArray): String

    /** Unpack ChannelPlanSyncAck PackLink frame (msg type 18). */
    external fun unpackChannelPlanSyncAckJson(wire: ByteArray): String

    /** Unpack SpoolReplay PackLink frame; JSON chunk metadata + payload hex. */
    external fun unpackSpoolReplayJson(wire: ByteArray): String

    /** Pack TimeSyncV1 from JSON; returns JSON with `wire_hex`. */
    external fun packTimeSyncJson(requestJson: String): String

    /** Unpack TimeSync PackLink frame. */
    external fun unpackTimeSyncJson(wire: ByteArray): String

    /** Unpack EventNotify PackLink frame with nested wolfPACK alert detail. */
    external fun unpackEventNotifyJson(wire: ByteArray): String

    /** Format one WiGLE WiFi CSV data line from observation JSON. */
    external fun formatWifiCsvLineJson(requestJson: String): String

    /** Format one WiGLE BLE CSV data line from observation JSON. */
    external fun formatBleCsvLineJson(requestJson: String): String

    /** Format one WiGLE cell CSV data line from observation JSON (blank Channel). */
    external fun formatCellCsvLineJson(requestJson: String): String

    /** Geofence monitor state-machine update. */
    external fun geofenceMonitorUpdateJson(requestJson: String): String

    /** Home zone latch update for wardrive suppression. */
    external fun homeZoneLatchUpdateJson(requestJson: String): String

    /** Stateless home zone point-in-circle test. */
    external fun homeZoneContainsJson(requestJson: String): String

    /** Bulk observation ingest (wifi + ble) with open enrichment session. */
    external fun ingestObservationBulkJson(wire: ByteArray, contextJson: String): String

    /** Unpack ProbeObservationBulk PackLink frame; JSON with `observations` array. */
    external fun probeObservationBulkJson(wire: ByteArray, sessionId: Long): String

    /** Batch cell tower enrich (one JNI call per GPS fix). */
    external fun ingestCellsJson(requestJson: String): String

    /** WiGLE CSV pre-header + column header. */
    external fun formatWigleHeadersJson(requestJson: String): String

    /** Sniffer display helpers (quiet_stale, last_seen_ago, etc.). */
    external fun snifferDisplayJson(requestJson: String): String

    // --- Binary JNI (map geometry + GPS timeline handle) ---

    /** Allocate a native GPS timeline handle; returns 0 on failure. */
    external fun gpsTimelineCreate(): Long

    /** Release a native GPS timeline handle. */
    external fun gpsTimelineDestroy(handle: Long)

    /** Append one GPS fix to a native timeline. */
    external fun gpsTimelineAppend(
        handle: Long,
        unixMs: Long,
        lat: Double,
        lon: Double,
        accuracyM: Float,
        altitudeM: Double,
        hasAltitude: Boolean,
        nowMs: Long,
    ): Boolean

    /** Prune timeline samples older than retention. */
    external fun gpsTimelinePrune(handle: Long, nowMs: Long)

    /** Merge route points into a native timeline. */
    external fun gpsTimelineMergeRoute(
        handle: Long,
        unixMs: LongArray,
        lats: DoubleArray,
        lons: DoubleArray,
        accuracyM: FloatArray,
        nowMs: Long,
    )

    /** JSON snapshot of timeline samples for ingest `gps_hist`. */
    external fun gpsTimelineToHistJson(handle: Long): String

    /** GPS fix accuracy usable for CSV gate. */
    external fun gpsTimelineIsUsable(accuracyM: Float): Boolean

    /** Fix timestamp within allowed skew. */
    external fun gpsTimelineIsValidFixTime(unixMs: Long, nowMs: Long): Boolean

    /** Replay bulk older than retention window. */
    external fun gpsTimelineIsStaleReplay(hdrScanEndUnixMs: Long, nowMs: Long): Boolean

    /** Douglas-Peucker simplification; returns interleaved [lat, lon, ...]. */
    external fun simplifyRouteArrays(
        lats: DoubleArray,
        lons: DoubleArray,
        maxPoints: Int,
    ): DoubleArray

    /** Cluster markers; returns Object[6] {lats, lons, counts, bestRssi, ssids, memberIndices}. */
    external fun clusterMapMarkersArrays(
        lats: DoubleArray,
        lons: DoubleArray,
        rssis: IntArray,
        ssids: Array<String>,
        bssids: Array<String>,
        signalKinds: Array<String>,
        zoom: Double,
        clusterZoomThreshold: Double,
        coordDecimals: Int,
    ): Array<Any>

    /** Coverage grid; returns Object[3] {fraction[1], unvisitedRows, unvisitedCols}. */
    external fun coverageGridArrays(
        latMin: Double,
        latMax: Double,
        lonMin: Double,
        lonMax: Double,
        rows: Int,
        cols: Int,
        routeLats: DoubleArray,
        routeLons: DoubleArray,
    ): Array<Any>

    // --- PackLink over gateway SoftAP (plaintext TCP) ---

    fun revisitRadiusMOrDefault(): Double =
        try {
            revisitRadiusM()
        } catch (_: UnsatisfiedLinkError) {
            50.0
        }

    /** Kotlin helper: parse PackLink frame JSON from JNI. */
    fun unpackPackLinkFrame(wire: ByteArray): PackLinkFrameResult =
        try {
            val root = JSONObject(unpackPackLinkFrameJson(wire))
            if (!root.optBoolean("ok", false)) {
                PackLinkFrameResult.Error(root.optString("error", "unpack failed"))
            } else {
                PackLinkFrameResult.Ok(
                    msgType = root.getInt("msg_type"),
                    msgTypeName = root.getString("msg_type_name"),
                    flags = root.getInt("flags"),
                    seq = root.getLong("seq"),
                    payloadHex = root.getString("payload_hex"),
                )
            }
        } catch (e: UnsatisfiedLinkError) {
            PackLinkFrameResult.Error("native library not loaded")
        } catch (e: Exception) {
            PackLinkFrameResult.Error(e.message ?: "json parse failed")
        }

    /** Open Rust enrichment session for a wardrive (empty MacTable / CellTable for this run). */
    fun openEnrichmentSession(sessionId: Long): Boolean =
        try {
            val req = JSONObject().put("session_id", sessionId)
            val root = JSONObject(enrichmentSessionOpenJson(req.toString()))
            root.optBoolean("ok", false)
        } catch (_: UnsatisfiedLinkError) {
            false
        } catch (_: Exception) {
            false
        }

    /** Close Rust enrichment session for a wardrive. */
    fun closeEnrichmentSession(sessionId: Long) {
        try {
            val req = JSONObject().put("session_id", sessionId)
            enrichmentSessionCloseJson(req.toString())
        } catch (_: UnsatisfiedLinkError) {
        } catch (_: Exception) {
        }
    }

    fun packPolicySync(requestJson: String): ByteArray? =
        try {
            val root = JSONObject(packPolicySyncJson(requestJson))
            if (!root.optBoolean("ok", false)) {
                null
            } else {
                hexToBytes(root.getString("wire_hex"))
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun packChannelPlanSync(requestJson: String): ByteArray? =
        try {
            val root = JSONObject(packChannelPlanSyncJson(requestJson))
            if (!root.optBoolean("ok", false)) null else hexToBytes(root.getString("wire_hex"))
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    /** Pack CaptureCmd for gateway control (job_id 1 = wifi_probe_pcap). */
    fun packCaptureCmd(jobId: Int = 1, durationSec: Int = 60, seq: Int = 0): ByteArray? =
        try {
            val req =
                JSONObject()
                    .put("job_id", jobId)
                    .put("duration_sec", durationSec)
                    .put("seq", seq)
            val root = JSONObject(packCaptureCmdJson(req.toString()))
            if (!root.optBoolean("ok", false)) null else hexToBytes(root.getString("wire_hex"))
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun unpackCaptureStatus(wire: ByteArray): net.wardriver.mobile.data.GatewayWifiRepository.CaptureStatus? {
        return try {
            val root = JSONObject(unpackCaptureStatusJson(wire))
            if (!root.optBoolean("ok", false)) {
                null
            } else {
                net.wardriver.mobile.data.GatewayWifiRepository.CaptureStatus(
                    active = root.getInt("active") != 0,
                    hold = root.getInt("hold") != 0,
                    remainingMs = root.getLong("remaining_ms"),
                    fragsRx = root.getLong("frags_rx"),
                    bytesWritten = root.getLong("bytes_written"),
                    errorCode = root.getInt("error_code"),
                )
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    /** Unpack `SnifferStatus` batch from gateway (PackLink msg type 8). */
    data class SnifferStatusBatch(
        val gatewayMonoMs: Long,
        val rows: List<net.wardriver.mobile.data.FleetRepository.SnifferStatusRow>,
    )

    fun unpackSnifferStatusBatch(wire: ByteArray): SnifferStatusBatch? {
        return try {
            val root = JSONObject(unpackSnifferStatusJson(wire))
            if (!root.optBoolean("ok", false)) return null
            val rows = root.optJSONArray("rows") ?: return SnifferStatusBatch(0L, emptyList())
            val out = mutableListOf<net.wardriver.mobile.data.FleetRepository.SnifferStatusRow>()
            for (i in 0 until rows.length()) {
                val row = rows.getJSONObject(i)
                out.add(
                    net.wardriver.mobile.data.FleetRepository.SnifferStatusRow(
                        snifferId = row.getInt("sniffer_id"),
                        mac = row.getString("mac"),
                        lastSeenMs = row.getLong("last_seen_ms"),
                        state = row.getInt("state"),
                        roleWire = row.getInt("role"),
                        cpuLoadPct = row.getInt("cpu_load_pct"),
                        heapAvailPct = row.getInt("heap_avail_pct"),
                        fleetSlot = row.getInt("fleet_slot"),
                        rebootCount = row.getInt("reboot_count"),
                        adoptionState = row.optInt("adoption_state", 0),
                        adoptProgress = row.optInt("adopt_progress", 0),
                        adoptReason = row.optInt("adopt_reason", 0),
                        fiveGhzCapable = row.optInt("five_ghz_capable", 0),
                        assignedBand = row.optInt("assigned_band", 0),
                        pinnedChannel = row.optInt("pinned_channel", 0),
                        assignedChanCount = row.optInt("assigned_chan_count", 0),
                        assignedChannelsPreview =
                            row.optJSONArray("assigned_channels_preview")?.let { arr ->
                                (0 until arr.length()).mapNotNull { i ->
                                    arr.optInt(i).takeIf { it > 0 }
                                }
                            } ?: emptyList(),
                    ),
                )
            }
            SnifferStatusBatch(
                gatewayMonoMs = root.optLong("gateway_mono_ms", 0L),
                rows = out,
            )
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    /** Unpack `SnifferStatus` batch from gateway (PackLink msg type 8). */
    fun unpackSnifferStatus(wire: ByteArray): List<net.wardriver.mobile.data.FleetRepository.SnifferStatusRow> =
        unpackSnifferStatusBatch(wire)?.rows ?: emptyList()

    data class PolicySyncAck(
        val policySeq: Long,
        val result: Int,
        val errorCode: Int,
    )

    data class ChannelPlanSyncAck(
        val planSeq: Long,
        val revision: Long,
        val result: Int,
        val errorCode: Int,
        val hopCycleMs: Long,
        val unassignedCount: Int,
    )

    fun unpackPolicySyncAck(wire: ByteArray): PolicySyncAck? {
        return try {
            val root = JSONObject(unpackPolicySyncAckJson(wire))
            if (!root.optBoolean("ok", false)) null
            else {
                PolicySyncAck(
                    policySeq = root.getLong("policy_seq"),
                    result = root.getInt("result"),
                    errorCode = root.getInt("error_code"),
                )
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    fun unpackChannelPlanSyncAck(wire: ByteArray): ChannelPlanSyncAck? {
        return try {
            val root = JSONObject(unpackChannelPlanSyncAckJson(wire))
            if (!root.optBoolean("ok", false)) null
            else {
                ChannelPlanSyncAck(
                    planSeq = root.getLong("plan_seq"),
                    revision = root.getLong("revision"),
                    result = root.getInt("result"),
                    errorCode = root.getInt("error_code"),
                    hopCycleMs = root.optLong("hop_cycle_ms", 0L),
                    unassignedCount = root.optInt("unassigned_count", 0),
                )
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    data class TimeSyncReply(
        val phoneUnixMs: Long,
        val phoneMonoMs: Long,
        val gatewayMonoMs: Long,
        val timeSetSeq: Long,
    )

    fun unpackTimeSync(wire: ByteArray): TimeSyncReply? {
        return try {
            val root = JSONObject(unpackTimeSyncJson(wire))
            if (!root.optBoolean("ok", false)) null
            else {
                TimeSyncReply(
                    phoneUnixMs = root.optLong("phone_unix_ms", 0L),
                    phoneMonoMs = root.optLong("phone_mono_ms", 0L),
                    gatewayMonoMs = root.optLong("gateway_mono_ms", 0L),
                    timeSetSeq = root.optLong("time_set_seq", 0L),
                )
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    fun unpackSpoolReplay(wire: ByteArray): net.wardriver.mobile.ingest.SpoolReplayChunk? =
        try {
            val root = JSONObject(unpackSpoolReplayJson(wire))
            if (!root.optBoolean("ok", false)) null
            else {
                net.wardriver.mobile.ingest.SpoolReplayChunk(
                    spoolOffset = root.getLong("spool_offset"),
                    chunkTotal = root.getLong("chunk_total"),
                    chunkIndex = root.getLong("chunk_index"),
                    payloadHex = root.getString("payload_hex"),
                )
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun packTimeSync(requestJson: String): ByteArray? =
        try {
            val root = JSONObject(packTimeSyncJson(requestJson))
            if (!root.optBoolean("ok", false)) null else hexToBytes(root.getString("wire_hex"))
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun unpackEventNotify(wire: ByteArray): net.wardriver.mobile.ingest.EventNotifyAlert? =
        try {
            val root = JSONObject(unpackEventNotifyJson(wire))
            if (!root.optBoolean("ok", false)) {
                null
            } else {
                val lat =
                    if (root.has("lat") && !root.isNull("lat")) root.getDouble("lat") else null
                val lon =
                    if (root.has("lon") && !root.isNull("lon")) root.getDouble("lon") else null
                val detectionMethodId =
                    if (root.has("detection_method_id") && !root.isNull("detection_method_id")) {
                        root.getInt("detection_method_id")
                    } else {
                        null
                    }
                net.wardriver.mobile.ingest.EventNotifyAlert(
                    kind = root.getString("kind"),
                    snifferId = root.getInt("sniffer_id"),
                    unixMs = root.getLong("unix_ms"),
                    detail = root.getString("detail"),
                    lat = lat,
                    lon = lon,
                    detectionMethodId = detectionMethodId,
                )
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun formatWifiCsvLine(requestJson: String): String? =
        try {
            val root = JSONObject(formatWifiCsvLineJson(requestJson))
            if (!root.optBoolean("ok", false)) null else root.getString("line")
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun formatBleCsvLine(requestJson: String): String? =
        try {
            val root = JSONObject(formatBleCsvLineJson(requestJson))
            if (!root.optBoolean("ok", false)) null else root.getString("line")
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun formatCellCsvLine(requestJson: String): String? =
        try {
            val root = JSONObject(formatCellCsvLineJson(requestJson))
            if (!root.optBoolean("ok", false)) null else root.getString("line")
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    /** Geofence monitor update; returns event + updated state. */
    fun geofenceMonitorUpdate(requestJson: String): JSONObject? =
        try {
            val root = JSONObject(geofenceMonitorUpdateJson(requestJson))
            if (!root.optBoolean("ok", false)) null else root
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    /** Home zone latch update; returns suppressed + latch state. */
    fun homeZoneLatchUpdate(requestJson: String): JSONObject? =
        try {
            val root = JSONObject(homeZoneLatchUpdateJson(requestJson))
            if (!root.optBoolean("ok", false)) null else root
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    /** Point-in-circle home zone test. */
    fun homeZoneContains(
        centerLat: Double,
        centerLon: Double,
        radiusM: Double,
        lat: Double,
        lon: Double,
    ): Boolean =
        try {
            val req =
                JSONObject()
                    .put("center_lat", centerLat)
                    .put("center_lon", centerLon)
                    .put("radius_m", radiusM)
                    .put("lat", lat)
                    .put("lon", lon)
            val root = JSONObject(homeZoneContainsJson(req.toString()))
            root.optBoolean("inside", false)
        } catch (_: UnsatisfiedLinkError) {
            false
        } catch (_: Exception) {
            false
        }

    data class BulkIngestResult(
        val stored: List<JSONObject>,
        val dropped: List<String>,
    )

    /** Bulk PackLink observation ingest via Rust enrichment session. */
    fun ingestObservationBulk(wire: ByteArray, contextJson: String): BulkIngestResult? {
        return try {
            val root = JSONObject(ingestObservationBulkJson(wire, contextJson))
            if (!root.optBoolean("ok", false)) {
                null
            } else {
                val storedArr = root.optJSONArray("stored") ?: JSONArray()
                val droppedArr = root.optJSONArray("dropped") ?: JSONArray()
                val stored = (0 until storedArr.length()).map { storedArr.getJSONObject(it) }
                val dropped =
                    (0 until droppedArr.length()).map { i ->
                        droppedArr.getJSONObject(i).optString("reason", "unknown")
                    }
                BulkIngestResult(stored = stored, dropped = dropped)
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    /** Batch cell tower enrich. */
    fun ingestCells(requestJson: String): BulkIngestResult? {
        return try {
            val root = JSONObject(ingestCellsJson(requestJson))
            if (!root.optBoolean("ok", false)) {
                null
            } else {
                val storedArr = root.optJSONArray("stored") ?: JSONArray()
                val droppedArr = root.optJSONArray("dropped") ?: JSONArray()
                val stored = (0 until storedArr.length()).map { storedArr.getJSONObject(it) }
                val dropped =
                    (0 until droppedArr.length()).map { i ->
                        droppedArr.getJSONObject(i).optString("reason", "unknown")
                    }
                BulkIngestResult(stored = stored, dropped = dropped)
            }
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }
    }

    /** Route simplification over parallel lat/lon arrays. */
    fun simplifyRouteFromArrays(
        lats: DoubleArray,
        lons: DoubleArray,
        maxPoints: Int,
    ): Pair<DoubleArray, DoubleArray>? =
        try {
            val interleaved = simplifyRouteArrays(lats, lons, maxPoints)
            val outLats = DoubleArray(interleaved.size / 2)
            val outLons = DoubleArray(interleaved.size / 2)
            var j = 0
            var i = 0
            while (i < interleaved.size) {
                outLats[j] = interleaved[i]
                outLons[j] = interleaved[i + 1]
                i += 2
                j++
            }
            outLats to outLons
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    data class ClusterArraysNative(
        val lats: DoubleArray,
        val lons: DoubleArray,
        val counts: IntArray,
        val bestRssi: IntArray,
        val labelSsids: Array<String>,
        val memberIndices: Array<IntArray>,
    )

    /** Grid/stack clustering without JSON round-trip. */
    fun clusterMapMarkersFromArrays(
        lats: DoubleArray,
        lons: DoubleArray,
        rssis: IntArray,
        ssids: Array<String>,
        bssids: Array<String>,
        signalKinds: Array<String>,
        zoom: Double,
        clusterZoomThreshold: Double,
        coordDecimals: Int = 5,
    ): ClusterArraysNative? =
        try {
            @Suppress("UNCHECKED_CAST")
            val parts = clusterMapMarkersArrays(
                lats, lons, rssis, ssids, bssids, signalKinds,
                zoom, clusterZoomThreshold, coordDecimals,
            ) as Array<Any>
            ClusterArraysNative(
                lats = parts[0] as DoubleArray,
                lons = parts[1] as DoubleArray,
                counts = parts[2] as IntArray,
                bestRssi = parts[3] as IntArray,
                labelSsids = parts[4] as Array<String>,
                memberIndices = (parts[5] as Array<*>).map { it as IntArray }.toTypedArray(),
            )
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    data class CoverageArraysNative(
        val fraction: Double,
        val unvisitedRows: IntArray,
        val unvisitedCols: IntArray,
    )

    /** Coverage fraction and unvisited cells in one native pass. */
    fun coverageGridFromArrays(
        latMin: Double,
        latMax: Double,
        lonMin: Double,
        lonMax: Double,
        rows: Int,
        cols: Int,
        routeLats: DoubleArray,
        routeLons: DoubleArray,
    ): CoverageArraysNative? =
        try {
            @Suppress("UNCHECKED_CAST")
            val parts = coverageGridArrays(
                latMin, latMax, lonMin, lonMax, rows, cols, routeLats, routeLons,
            ) as Array<Any>
            val fractionArr = parts[0] as DoubleArray
            CoverageArraysNative(
                fraction = fractionArr[0],
                unvisitedRows = parts[1] as IntArray,
                unvisitedCols = parts[2] as IntArray,
            )
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    fun gpsTimelineIsUsableOrFallback(accuracyM: Float): Boolean =
        try {
            gpsTimelineIsUsable(accuracyM)
        } catch (_: UnsatisfiedLinkError) {
            accuracyM > 0f && accuracyM <= 25f
        }

    fun gpsTimelineIsValidFixTimeOrFallback(unixMs: Long, nowMs: Long): Boolean =
        try {
            gpsTimelineIsValidFixTime(unixMs, nowMs)
        } catch (_: UnsatisfiedLinkError) {
            unixMs > 0L && unixMs <= nowMs + 60_000L
        }

    fun gpsTimelineIsStaleReplayOrFallback(hdrScanEndUnixMs: Long, nowMs: Long): Boolean =
        try {
            gpsTimelineIsStaleReplay(hdrScanEndUnixMs, nowMs)
        } catch (_: UnsatisfiedLinkError) {
            hdrScanEndUnixMs > 0L &&
                (if (nowMs >= hdrScanEndUnixMs) nowMs - hdrScanEndUnixMs else 0L) > 300_000L
        }

    /** WiGLE pre-header + column header block. */
    fun formatWigleHeaders(deviceJson: JSONObject): String? =
        try {
            val root = JSONObject(formatWigleHeadersJson(deviceJson.toString()))
            if (!root.optBoolean("ok", false)) null else root.getString("headers")
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    /** Sniffer display helper dispatch. */
    fun snifferDisplay(op: String, requestFields: JSONObject): String? =
        try {
            val req = JSONObject(requestFields.toString()).put("op", op)
            val root = JSONObject(snifferDisplayJson(req.toString()))
            if (!root.optBoolean("ok", false)) null else root.getString("value")
        } catch (_: UnsatisfiedLinkError) {
            null
        } catch (_: Exception) {
            null
        }

    private fun hexToBytes(hex: String): ByteArray {
        val clean = hex.trim()
        val out = ByteArray(clean.length / 2)
        var i = 0
        while (i < clean.length) {
            out[i / 2] = clean.substring(i, i + 2).toInt(16).toByte()
            i += 2
        }
        return out
    }

    sealed interface PackLinkFrameResult {
        data class Ok(
            val msgType: Int,
            val msgTypeName: String,
            val flags: Int,
            val seq: Long,
            val payloadHex: String,
        ) : PackLinkFrameResult

        data class Error(val message: String) : PackLinkFrameResult
    }
}
