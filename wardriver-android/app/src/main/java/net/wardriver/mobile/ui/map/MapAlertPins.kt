package net.wardriver.mobile.ui.map

import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.db.DetectionEntity
import org.json.JSONObject

/** Parsed Remote ID fields from persisted JSON `detail`. */
data class RemoteIdDetail(
    val mac: String?,
    val channel: Int?,
    val rssi: Int?,
    val method: Int,
    val signalKind: Int? = null,
    val uasId: String? = null,
    val idType: Int? = null,
    val uaType: Int? = null,
    val status: Int? = null,
    val headingDeg: Int? = null,
    val speedHMps: Float? = null,
    val altGeoM: Float? = null,
    val heightAglM: Float? = null,
    val heightType: Int? = null,
    val horizAccuracy: Int? = null,
    val vertAccuracy: Int? = null,
    val speedAccuracy: Int? = null,
    val baroAccuracy: Int? = null,
    val tsAccuracy: Int? = null,
    val timestampSec: Float? = null,
    val operatorId: String? = null,
    val operatorIdType: Int? = null,
    val operatorLat: Double? = null,
    val operatorLon: Double? = null,
    val operatorLocationType: Int? = null,
)

/** Build map alert pins from detection rows with stored or parsed GPS coordinates. */
fun buildAlertPins(detections: List<DetectionEntity>): List<AlertPin> =
    detections.mapNotNull { det ->
        val coords = detectionCoords(det) ?: return@mapNotNull null
        val remoteId = if (det.kind == AlertKind.REMOTE_ID) parseRemoteIdDetail(det.detail) else null
        AlertPin(
            id = "${det.kind}_${det.id}",
            lat = coords.first,
            lon = coords.second,
            kind = det.kind,
            title = alertTitle(det.kind, det.detectionMethodId ?: parseDetectionMethodId(det.detail)),
            detail = det.detail,
            unixMs = det.unixMs,
            detectionMethodId = det.detectionMethodId ?: parseDetectionMethodId(det.detail),
            remoteId = remoteId,
        )
    }

internal fun detectionCoords(det: DetectionEntity): Pair<Double, Double>? {
    if (det.lat != null && det.lon != null) {
        return det.lat to det.lon
    }
    return parseDetectionCoords(det)
}

internal fun parseDetectionCoords(det: DetectionEntity): Pair<Double, Double>? {
    return try {
        val json = JSONObject(det.detail)
        if (json.has("lat") && json.has("lon")) {
            json.getDouble("lat") to json.getDouble("lon")
        } else {
            null
        }
    } catch (_: Exception) {
        parseLatLonDetail(det.detail)
    }
}

internal fun parseLatLonDetail(detail: String): Pair<Double, Double>? {
    val lat = Regex("""lat=([-\d.]+)""").find(detail)?.groupValues?.get(1)?.toDoubleOrNull()
    val lon = Regex("""lon=([-\d.]+)""").find(detail)?.groupValues?.get(1)?.toDoubleOrNull()
    return if (lat != null && lon != null) lat to lon else null
}

/** Parse wolfPACK `detection_method_id` from JSON or legacy detail text. */
internal fun parseDetectionMethodId(detail: String): Int? {
    val trimmed = detail.trim()
    if (trimmed.startsWith("{")) {
        try {
            val json = JSONObject(trimmed)
            if (json.has("method") && !json.isNull("method")) {
                return json.getInt("method")
            }
        } catch (_: Exception) {
            // fall through to legacy tokens
        }
        Regex(""""method"\s*:\s*(\d+)""").find(trimmed)?.groupValues?.get(1)?.toIntOrNull()?.let {
            return it
        }
    }
    Regex("""method=(\d+)""").find(detail)?.groupValues?.get(1)?.toIntOrNull()?.let {
        return it
    }
    return Regex("""detection_method_id=(\d+)""").find(detail)?.groupValues?.get(1)?.toIntOrNull()
}

internal fun parseRemoteIdDetail(detail: String): RemoteIdDetail? {
  return try {
        val json = JSONObject(detail.trim())
        RemoteIdDetail(
            mac = json.optStringOrNull("mac"),
            channel = json.optIntOrNull("channel"),
            rssi = json.optIntOrNull("rssi"),
            method = json.optInt("method", 0),
            signalKind = json.optIntOrNull("signal_kind"),
            uasId = json.optStringOrNull("uas_id"),
            idType = json.optIntOrNull("id_type"),
            uaType = json.optIntOrNull("ua_type"),
            status = json.optIntOrNull("status"),
            headingDeg = json.optIntOrNull("heading_deg"),
            speedHMps = json.optDoubleOrNull("speed_h_mps")?.toFloat(),
            altGeoM = json.optDoubleOrNull("alt_geo_m")?.toFloat(),
            heightAglM = json.optDoubleOrNull("height_agl_m")?.toFloat(),
            heightType = json.optIntOrNull("height_type"),
            horizAccuracy = json.optIntOrNull("horiz_accuracy"),
            vertAccuracy = json.optIntOrNull("vert_accuracy"),
            speedAccuracy = json.optIntOrNull("speed_accuracy"),
            baroAccuracy = json.optIntOrNull("baro_accuracy"),
            tsAccuracy = json.optIntOrNull("ts_accuracy"),
            timestampSec = json.optDoubleOrNull("timestamp_sec")?.toFloat(),
            operatorId = json.optStringOrNull("operator_id"),
            operatorIdType = json.optIntOrNull("operator_id_type"),
            operatorLat = json.optDoubleOrNull("operator_lat"),
            operatorLon = json.optDoubleOrNull("operator_lon"),
            operatorLocationType = json.optIntOrNull("operator_location_type"),
        )
    } catch (_: Exception) {
        null
    }
}

private fun JSONObject.optIntOrNull(key: String): Int? =
    if (has(key) && !isNull(key)) optInt(key) else null

private fun JSONObject.optDoubleOrNull(key: String): Double? =
    if (has(key) && !isNull(key)) optDouble(key) else null

private fun JSONObject.optStringOrNull(key: String): String? {
    if (!has(key) || isNull(key)) return null
    val value = optString(key)
    return value.ifEmpty { null }
}

internal fun alertTitle(kind: String, detectionMethodId: Int? = null): String {
    if (kind == AlertKind.REMOTE_ID) return "Remote ID"
    when (detectionMethodId) {
        4, 35 -> return "Axon"
        5, 36 -> return "i-PRO"
    }
    val base =
        when {
            kind.contains("flock", ignoreCase = true) || kind.contains("detection", ignoreCase = true) -> "Detection alert"
            kind.contains("ssid", ignoreCase = true) -> "SSID watch"
            else -> "Detection"
        }
    return if (detectionMethodId != null &&
        (kind.contains("flock", ignoreCase = true) || kind.contains("detection", ignoreCase = true))) {
        "$base · ID $detectionMethodId"
    } else {
        base
    }
}

/** Human-readable label for a detection `detection_method_id` (see docs/DETECTION_METHODS.md). */
fun detectionMethodLabel(methodId: Int): String =
    when (methodId) {
        1 -> "Wildcard probe + Flock OUI"
        2 -> "Wildcard probe + IE signature + Flock OUI"
        3 -> "Wildcard probe + IE signature (any MAC)"
        4 -> "Axon OUI (Wi-Fi)"
        5 -> "i-PRO OUI (Wi-Fi)"
        32 -> "Xuntong manufacturer ID"
        33 -> "Advertised name keyword"
        34 -> "MAC OUI (infrastructure)"
        35 -> "Axon OUI"
        36 -> "i-PRO OUI"
        else -> "Method $methodId"
    }

/** Clusters hidden while a Remote ID pin is selected. */
internal fun remoteIdFocusedClusters(
    clusters: List<ApCluster>,
    selected: AlertPin?,
): List<ApCluster> = if (selected?.kind == AlertKind.REMOTE_ID) emptyList() else clusters

/** Only the selected Remote ID pin is shown while its sheet is open. */
internal fun remoteIdFocusedAlertPins(
    pins: List<AlertPin>,
    selected: AlertPin?,
): List<AlertPin> = if (selected?.kind == AlertKind.REMOTE_ID) listOf(selected) else pins

/** Valid operator position for map overlay (non-zero lat/lon). */
internal fun remoteIdOperatorPoint(pin: AlertPin): org.osmdroid.util.GeoPoint? {
    val rid = pin.remoteId ?: return null
    val lat = rid.operatorLat ?: return null
    val lon = rid.operatorLon ?: return null
    if (lat == 0.0 && lon == 0.0) return null
    if (lat !in -90.0..90.0 || lon !in -180.0..180.0) return null
    return org.osmdroid.util.GeoPoint(lat, lon)
}
