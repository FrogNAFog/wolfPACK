package net.wardriver.mobile.data

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import net.wardriver.mobile.db.DetectionDao
import net.wardriver.mobile.db.NetworkSummaryDao
import net.wardriver.mobile.db.ProbeObservationDao
import net.wardriver.mobile.db.RouteDao
import net.wardriver.mobile.db.SessionDao
import net.wardriver.mobile.db.SessionMapMarkerDao
import net.wardriver.mobile.db.DetectionEntity
import net.wardriver.mobile.db.NetworkSummaryEntity
import net.wardriver.mobile.db.ProbeObservationEntity
import net.wardriver.mobile.db.RoutePointEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionMapMarkerEntity
import net.wardriver.mobile.db.SessionWithCount
import net.wardriver.mobile.ui.map.AlertPin
import net.wardriver.mobile.ui.map.ApMarker
import net.wardriver.mobile.ui.map.GeofenceBounds
import net.wardriver.mobile.ui.map.MapViewport
import net.wardriver.mobile.ui.map.buildAlertPins

/** Route point as a UI model (position + monotonically increasing id for dedup). */
data class RoutePoint(
    val id: Long,
    val lat: Double,
    val lon: Double,
)

/**
 * Owns Room DAO access for the map screen and maps entities to UI models
 * ([ApMarker], [AlertPin], [RoutePoint], [GeofenceBounds]). [MapViewModel]
 * consumes these UI models and never touches Room entities directly.
 */
class MapStore(
    private val sessionDao: SessionDao,
    private val routeDao: RouteDao,
    private val detectionDao: DetectionDao,
    private val sessionMapMarkerDao: SessionMapMarkerDao,
    private val probeObservationDao: ProbeObservationDao,
    private val networkSummaryDao: NetworkSummaryDao,
) {
    // --- AP markers ---

    /** Session-scoped AP markers: session map markers + probe observations in the viewport. */
    fun apMarkersForSession(
        sessionId: Long,
        vp: MapViewport,
    ): Flow<List<ApMarker>> =
        combineViewport(
            sessionMapMarkerDao.inViewportFlow(sessionId, vp.south, vp.north, vp.west, vp.east)
                .map { markers -> markers.map { it.toApMarker() } },
            probeObservationDao.inViewportFlow(sessionId, vp.south, vp.north, vp.west, vp.east)
                .map { probes -> probes.map { it.toApMarker() } },
        )

    /** All-devices AP markers from the global network-summary table in the viewport. */
    fun apMarkersAllDevices(
        vp: MapViewport,
    ): Flow<List<ApMarker>> =
        networkSummaryDao.inViewportFlow(vp.south, vp.north, vp.west, vp.east)
            .map { summaries -> summaries.map { it.toApMarkerFromSummary() } }

    // --- Alert pins ---

    /** Detection rows for a session, mapped to alert pins. */
    fun alertPinsForSession(
        sessionId: Long,
    ): Flow<List<AlertPin>> =
        detectionDao.forSessionFlow(sessionId).map { detections -> buildAlertPins(detections) }

    /** Detection rows in the viewport (all-devices scope), mapped to alert pins. */
    fun alertPinsInViewport(
        vp: MapViewport,
    ): Flow<List<AlertPin>> =
        detectionDao
            .detectionsInViewportFlow(vp.south, vp.north, vp.west, vp.east)
            .map { detections -> buildAlertPins(detections) }

    // --- Route ---

    /** Initial route points for a session. */
    suspend fun initialRoutePoints(
        sessionId: Long,
    ): List<RoutePoint> =
        routeDao.forSession(sessionId).map { it.toRoutePoint() }

    /** Latest route point appended after the initial batch (for incremental updates). */
    fun latestRoutePointFlow(
        sessionId: Long,
    ): Flow<RoutePoint?> =
        routeDao.latestForSessionFlow(sessionId).map { it?.toRoutePoint() }

    // --- Geofence ---

    /** Geofence bounds for a session row (null when absent or degenerate). */
    fun sessionGeofenceFlow(
        sessionId: Long,
    ): Flow<GeofenceBounds?> =
        sessionDao.getFlow(sessionId).map { it?.toGeofenceBounds() }

    // --- Sessions ---

    /** Most recent session id (map fallback when no active session). */
    suspend fun latestSessionId(): Long? = sessionDao.getLatest()?.id

    /** All sessions with observation counts (session picker). */
    suspend fun allSessionsWithCounts(): List<SessionWithCount> =
        sessionDao.allSessionsWithCounts()

    private fun combineViewport(
        a: Flow<List<ApMarker>>,
        b: Flow<List<ApMarker>>,
    ): Flow<List<ApMarker>> =
        combine(a, b) { apMarkers, probeMarkers -> apMarkers + probeMarkers }
}

// --- Entity -> UI mappers (moved from MapViewModel) ---

internal fun NetworkSummaryEntity.toApMarkerFromSummary(): ApMarker =
    ApMarker(
        lat = firstLat,
        lon = firstLon,
        ssid = ssid,
        rssi = lastRssi,
        bssid = bssid,
        signalKind = lastSignalKind,
    )

internal fun SessionMapMarkerEntity.toApMarker(): ApMarker =
    ApMarker(
        lat = lat,
        lon = lon,
        ssid = ssid,
        rssi = rssi,
        bssid = bssid,
        signalKind = signalKind,
    )

internal fun ProbeObservationEntity.toApMarker(): ApMarker =
    ApMarker(
        lat = lat,
        lon = lon,
        ssid = ssid,
        rssi = rssi,
        bssid = srcMac,
        signalKind = ApMarkerKind.PROBE_REQ,
    )

internal fun RoutePointEntity.toRoutePoint(): RoutePoint = RoutePoint(id, lat, lon)

internal fun SessionEntity.toGeofenceBounds(): GeofenceBounds? {
    val latMin = geofenceLatMin
    val latMax = geofenceLatMax
    val lonMin = geofenceLonMin
    val lonMax = geofenceLonMax
    if (latMin == null || latMax == null || lonMin == null || lonMax == null) return null
    if (latMin >= latMax || lonMin >= lonMax) return null
    return GeofenceBounds(latMin, latMax, lonMin, lonMax)
}
