package net.wardriver.mobile.ingest

import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.db.RoutePointEntity
import org.json.JSONArray
import org.json.JSONObject

/** One GPS sample used for PackLink observation anchor matching. */
data class GpsSample(
    val unixMs: Long,
    val lat: Double,
    val lon: Double,
    val altitudeM: Double?,
    val accuracyM: Float,
)

/**
 * Control-parity GPS timeline backed by Rust [`gps_timeline`].
 * Native handle owns timeline state; JSON snapshot only at bulk ingest.
 */
class GpsTimeline {
    private var handle: Long = createHandle()

    private fun createHandle(): Long =
        try {
            NativeCore.gpsTimelineCreate()
        } catch (_: UnsatisfiedLinkError) {
            0L
        }

    /** Release native timeline resources. */
    fun close() {
        if (handle != 0L) {
            try {
                NativeCore.gpsTimelineDestroy(handle)
            } catch (_: UnsatisfiedLinkError) {
            }
            handle = 0L
        }
    }

    fun append(
        unixMs: Long,
        lat: Double,
        lon: Double,
        accuracyM: Float,
        altitudeM: Double?,
        nowMs: Long = WardriveClock.nowUnixMs(),
    ): Boolean {
        if (handle != 0L) {
            return try {
                NativeCore.gpsTimelineAppend(
                    handle,
                    unixMs,
                    lat,
                    lon,
                    accuracyM,
                    altitudeM ?: 0.0,
                    altitudeM != null,
                    nowMs,
                )
            } catch (_: UnsatisfiedLinkError) {
                appendFallback(unixMs, lat, lon, accuracyM, altitudeM, nowMs)
            }
        }
        return appendFallback(unixMs, lat, lon, accuracyM, altitudeM, nowMs)
    }

    fun mergeRoutePoints(points: List<RoutePointEntity>, nowMs: Long = WardriveClock.nowUnixMs()) {
        if (points.isEmpty()) return
        if (handle != 0L) {
            val unix = LongArray(points.size)
            val lats = DoubleArray(points.size)
            val lons = DoubleArray(points.size)
            val acc = FloatArray(points.size)
            var n = 0
            for (point in points) {
                val accuracy = point.accuracyM ?: continue
                unix[n] = point.unixMs
                lats[n] = point.lat
                lons[n] = point.lon
                acc[n] = accuracy
                n++
            }
            if (n == 0) return
            try {
                NativeCore.gpsTimelineMergeRoute(
                    handle,
                    if (n == unix.size) unix else unix.copyOf(n),
                    if (n == lats.size) lats else lats.copyOf(n),
                    if (n == lons.size) lons else lons.copyOf(n),
                    if (n == acc.size) acc else acc.copyOf(n),
                    nowMs,
                )
                return
            } catch (_: UnsatisfiedLinkError) {
            }
        }
        for (point in points) {
            val accuracy = point.accuracyM ?: continue
            appendFallback(point.unixMs, point.lat, point.lon, accuracy, null, nowMs)
        }
    }

    fun prune(nowMs: Long = WardriveClock.nowUnixMs()) {
        if (handle != 0L) {
            try {
                NativeCore.gpsTimelinePrune(handle, nowMs)
                return
            } catch (_: UnsatisfiedLinkError) {
            }
        }
        pruneFallback(nowMs)
    }

    fun isUsable(accuracyM: Float): Boolean = NativeCore.gpsTimelineIsUsableOrFallback(accuracyM)

    fun toJsonArray(): JSONArray {
        if (handle != 0L) {
            return try {
                val root = JSONObject(NativeCore.gpsTimelineToHistJson(handle))
                root.optJSONArray("gps_hist") ?: JSONArray()
            } catch (_: UnsatisfiedLinkError) {
                fallbackTimeline
            } catch (_: Exception) {
                fallbackTimeline
            }
        }
        return fallbackTimeline
    }

    fun size(): Int = toJsonArray().length()

    private var fallbackTimeline = JSONArray()

    /** Host/unit-test fallback when JNI is absent. */
    private fun appendFallback(
        unixMs: Long,
        lat: Double,
        lon: Double,
        accuracyM: Float,
        altitudeM: Double?,
        nowMs: Long,
    ): Boolean {
        if (!isValidFixTime(unixMs, nowMs)) return false
        if (accuracyM <= 0f || accuracyM > ObservationIngestor.ROUTE_MAX_ACCURACY_M) return false
        val fix =
            JSONObject().apply {
                put("unix_ms", unixMs)
                put("lat", lat)
                put("lon", lon)
                put("accuracy_m", accuracyM)
                altitudeM?.let { put("altitude_m", it) }
            }
        val last =
            if (fallbackTimeline.length() > 0) {
                fallbackTimeline.getJSONObject(fallbackTimeline.length() - 1)
            } else {
                null
            }
        if (last != null && last.getLong("unix_ms") == unixMs) {
            fallbackTimeline.put(fallbackTimeline.length() - 1, fix)
        } else {
            fallbackTimeline.put(fix)
        }
        return true
    }

    private fun pruneFallback(nowMs: Long) {
        val cutoff = nowMs - GPS_HIST_RETENTION_MS
        val kept = JSONArray()
        for (i in 0 until fallbackTimeline.length()) {
            val item = fallbackTimeline.getJSONObject(i)
            if (item.getLong("unix_ms") >= cutoff) {
                kept.put(item)
            }
        }
        fallbackTimeline = kept
    }

    companion object {
        /** Matches control [`GPS_HIST_RETENTION_MS`] (five minutes). */
        const val GPS_HIST_RETENTION_MS = 300_000L

        /** Shared anchor match window from `wardriver_core::gps_anchor`. */
        const val MAX_FIX_TIMELINE_DELTA_MS = 2_000L

        fun isValidFixTime(unixMs: Long, nowMs: Long = WardriveClock.nowUnixMs()): Boolean {
            if (unixMs <= 0L) return false
            return NativeCore.gpsTimelineIsValidFixTimeOrFallback(unixMs, nowMs)
        }

        fun isStaleReplay(hdrScanEndUnixMs: Long, nowMs: Long = WardriveClock.nowUnixMs()): Boolean =
            NativeCore.gpsTimelineIsStaleReplayOrFallback(hdrScanEndUnixMs, nowMs)
    }
}
