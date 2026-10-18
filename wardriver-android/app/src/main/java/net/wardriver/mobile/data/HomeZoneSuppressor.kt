package net.wardriver.mobile.data

import net.wardriver.mobile.NativeCore
import org.json.JSONObject

/**
 * Latched home-zone suppression for wardrive (mirrors control `fleet_loop` home geofence latch).
 * State machine lives in Rust [`home_zone_latch`].
 */
class HomeZoneSuppressor {
    private var insideHomeZonePrev = false
    private var outsideGpsOkSinceMs = 0L

    /** Current `wardrive_suppressed` for PolicySync when home zone is enabled. */
    fun wardriveSuppressed(
        snapshot: AppPreferences.PrefsSnapshot,
        lat: Double?,
        lon: Double?,
        gpsOk: Boolean,
        nowMs: Long = System.currentTimeMillis(),
    ): Boolean {
        if (!snapshot.homeZoneEnabled) {
            insideHomeZonePrev = false
            outsideGpsOkSinceMs = 0L
            return false
        }
        val centerLat = snapshot.homeZoneLat ?: return false
        val centerLon = snapshot.homeZoneLon ?: return false
        val radiusM = snapshot.homeZoneRadiusM

        val req =
            JSONObject()
                .put("enabled", true)
                .put("center_lat", centerLat)
                .put("center_lon", centerLon)
                .put("radius_m", radiusM)
                .put("inside_prev", insideHomeZonePrev)
                .put("outside_since_ms", outsideGpsOkSinceMs)
                .put("lat", lat ?: JSONObject.NULL)
                .put("lon", lon ?: JSONObject.NULL)
                .put("gps_ok", gpsOk)
                .put("now_ms", nowMs)
        val out = NativeCore.homeZoneLatchUpdate(req.toString())
        if (out != null) {
            insideHomeZonePrev = out.optBoolean("inside_prev", false)
            outsideGpsOkSinceMs = out.optLong("outside_since_ms", 0L)
            return out.optBoolean("suppressed", false)
        }
        return false
    }
}
