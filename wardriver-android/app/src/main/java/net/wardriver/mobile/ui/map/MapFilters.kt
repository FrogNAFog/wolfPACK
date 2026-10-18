package net.wardriver.mobile.ui.map

import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.data.ApMarkerKind

/** Map AP source: current session observations or global device history. */
enum class MapDataScope {
    Session,
    AllDevices,
}

/** Layer toggles for AP markers and detection alert pins. */
data class MapLayerFilters(
    val wifi: Boolean = true,
    val ble: Boolean = true,
    val probe: Boolean = true,
    val alerts: Boolean = true,
    val alertDetectionWifi: Boolean = true,
    val alertDetectionBle: Boolean = true,
    val alertSsidWatch: Boolean = true,
    val alertRemoteId: Boolean = true,
    val alertAxon: Boolean = true,
    val alertIPro: Boolean = true,
) {
    fun filterApMarkers(markers: List<ApMarker>): List<ApMarker> =
        markers.filter { ap ->
            when (ap.signalKind.uppercase()) {
                "WIFI" -> wifi
                "BLE" -> ble
                ApMarkerKind.PROBE_REQ -> probe
                else -> false
            }
        }

    fun filterAlertPins(pins: List<AlertPin>): List<AlertPin> =
        pins.filter { pin ->
            if (!alerts) return@filter false
            when (pin.kind) {
                in AlertKind.WIFI_KINDS -> alertDetectionWifi
                in AlertKind.BLE_KINDS -> alertDetectionBle
                AlertKind.SSID_WATCH -> alertSsidWatch
                AlertKind.REMOTE_ID -> alertRemoteId
                AlertKind.AXON -> alertAxon
                AlertKind.I_PRO -> alertIPro
                else -> true
            }
        }

    /** Number of individually enabled alert type toggles. */
    val alertEnabledCount: Int
        get() = buildList {
            if (alertDetectionWifi) add(1)
            if (alertDetectionBle) add(1)
            if (alertSsidWatch) add(1)
            if (alertRemoteId) add(1)
            if (alertAxon) add(1)
            if (alertIPro) add(1)
        }.size
}

/** Visible map bounding box (degrees). */
data class MapViewport(
    val south: Double,
    val north: Double,
    val west: Double,
    val east: Double,
)
