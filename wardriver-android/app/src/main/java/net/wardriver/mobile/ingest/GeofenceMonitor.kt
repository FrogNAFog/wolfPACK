package net.wardriver.mobile.ingest

import net.wardriver.mobile.NativeCore
import org.json.JSONObject

/**
 * Tracks axis-aligned geofence square transitions for leave-square alerts.
 * State machine lives in Rust [`geofence`]; Kotlin holds `wasInside` latch.
 */
class GeofenceMonitor(
    private val latMin: Double,
    private val latMax: Double,
    private val lonMin: Double,
    private val lonMax: Double,
    val enabled: Boolean,
) {
    private var wasInside: Boolean? = null

    fun contains(lat: Double, lon: Double): Boolean {
        if (!enabled) return true
        val req =
            JSONObject()
                .put("enabled", true)
                .put("lat_min", latMin)
                .put("lat_max", latMax)
                .put("lon_min", lonMin)
                .put("lon_max", lonMax)
                .put("lat", lat)
                .put("lon", lon)
                .put("was_inside", JSONObject.NULL)
        val out = NativeCore.geofenceMonitorUpdate(req.toString())
        return out?.optBoolean("inside", false)
            ?: (lat in latMin..latMax && lon in lonMin..lonMax)
    }

    /** Returns [GeofenceEvent.Left] on first exit after being inside. */
    fun update(lat: Double, lon: Double): GeofenceEvent {
        if (!enabled) return GeofenceEvent.None
        val req =
            JSONObject()
                .put("enabled", true)
                .put("lat_min", latMin)
                .put("lat_max", latMax)
                .put("lon_min", lonMin)
                .put("lon_max", lonMax)
                .put("lat", lat)
                .put("lon", lon)
                .put("was_inside", wasInside ?: JSONObject.NULL)
        val out = NativeCore.geofenceMonitorUpdate(req.toString())
        if (out != null) {
            wasInside =
                if (out.isNull("was_inside")) null else out.optBoolean("was_inside", false)
            return when (out.optString("event", "None")) {
                "Entered" -> GeofenceEvent.Entered
                "Left" -> GeofenceEvent.Left
                else -> GeofenceEvent.None
            }
        }
        // JNI absent — Kotlin fallback
        val inside = lat in latMin..latMax && lon in lonMin..lonMax
        val event =
            when {
                wasInside == null && inside -> GeofenceEvent.Entered
                wasInside == true && !inside -> GeofenceEvent.Left
                wasInside == false && inside -> GeofenceEvent.Entered
                else -> GeofenceEvent.None
            }
        wasInside = inside
        return event
    }

    enum class GeofenceEvent {
        Entered,
        Left,
        None,
    }
}
