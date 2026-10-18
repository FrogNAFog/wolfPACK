package net.wardriver.mobile.data

/**
 * Canonical alert `kind` strings shared with the wolfPACK firmware.
 * Legacy `FLOCK_*` names are still present in persisted rows written before
 * the Detection rename.
 */
object AlertKind {
    const val FLOCK_WIFI = "FLOCK_WIFI"
    const val DETECTION_WIFI = "DETECTION_WIFI"
    const val FLOCK_BLE = "FLOCK_BLE"
    const val DETECTION_BLE = "DETECTION_BLE"
    const val REMOTE_ID = "REMOTE_ID"

    /** WiFi camera alerts (legacy + renamed). */
    val WIFI_KINDS = listOf(FLOCK_WIFI, DETECTION_WIFI)

    /** BLE camera alerts (legacy + renamed). */
    val BLE_KINDS = listOf(FLOCK_BLE, DETECTION_BLE)

    /** Camera detection alerts routed through [AlertRouter]. */
    val DETECTION_KINDS = WIFI_KINDS + BLE_KINDS

    /** Substring tokens for legacy row heuristics (contains checks). */
    const val FLOCK = "FLOCK"
    const val DETECTION = "DETECTION"
    const val GEOFENCE = "GEOFENCE"

    /** Non-camera alert kinds (equality matching in filter/detail UI). */
    const val SSID_WATCH = "SSID_WATCH"
    const val AXON = "AXON"
    const val I_PRO = "I-PRO"

    /** True for camera detection kinds, including legacy `FLOCK_*` spellings. */
    fun isDetectionKind(kind: String): Boolean =
        kind in DETECTION_KINDS ||
            kind.contains(FLOCK, ignoreCase = true) ||
            kind.contains(DETECTION, ignoreCase = true)
}
