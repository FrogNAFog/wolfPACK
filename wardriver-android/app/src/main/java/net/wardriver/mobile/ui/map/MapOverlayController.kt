package net.wardriver.mobile.ui.map

import android.os.Handler
import android.os.Looper
import net.wardriver.mobile.ui.theme.PackColor
import org.osmdroid.util.GeoPoint
import org.osmdroid.views.MapView
import org.osmdroid.views.overlay.Marker
import org.osmdroid.views.overlay.Overlay
import org.osmdroid.views.overlay.Polygon
import org.osmdroid.views.overlay.Polyline

/**
 * Incrementally syncs map overlays without tearing down the full layer stack on each update.
 * MyLocation overlay is owned by [MapScreen] and is never touched here.
 */
class MapOverlayController(private val map: MapView) {
    private var routePolyline: Polyline? = null
    private var geofencePolygon: Polygon? = null
    private var previewGeofencePolygon: Polygon? = null
    private val coveragePolygons = mutableMapOf<String, Polygon>()
    private val clusterMarkers = mutableMapOf<String, Marker>()
    private val alertMarkers = mutableMapOf<String, Marker>()
    private var operatorMarker: Marker? = null
    private var invalidatePending = false
    private val mainHandler = Handler(Looper.getMainLooper())

    private val clusterClickListener =
        Marker.OnMarkerClickListener { marker, _ ->
            val cluster = marker.relatedObject as? ApCluster ?: return@OnMarkerClickListener false
            clusterClickHandler?.invoke(cluster)
            true
        }

    private val alertClickListener =
        Marker.OnMarkerClickListener { marker, _ ->
            val pin = marker.relatedObject as? AlertPin ?: return@OnMarkerClickListener false
            alertClickHandler?.invoke(pin)
            true
        }

    private var clusterClickHandler: ((ApCluster) -> Unit)? = null
    private var alertClickHandler: ((AlertPin) -> Unit)? = null

    fun syncRoute(points: List<GeoPoint>) {
        if (points.size < 2) {
            routePolyline?.let { removeOverlay(it) }
            routePolyline = null
            requestInvalidate()
            return
        }
        val polyline =
            routePolyline ?: Polyline().apply {
                outlinePaint.color = PackColor.argb(PackColor.Cyan)
                outlinePaint.strokeWidth = 6f
                map.overlays.add(this)
                routePolyline = this
            }
        polyline.setPoints(points)
        requestInvalidate()
    }

    fun syncGeofence(bounds: GeofenceBounds?) {
        if (bounds == null) {
            geofencePolygon?.let { removeOverlay(it) }
            geofencePolygon = null
            requestInvalidate()
            return
        }
        val polygon =
            geofencePolygon ?: Polygon().apply {
                map.overlays.add(this)
                geofencePolygon = this
            }
        applyGeofenceStyle(polygon, preview = false)
        polygon.setPoints(geofencePoints(bounds))
        requestInvalidate()
    }

    fun syncGeofencePreview(bounds: GeofenceBounds?) {
        if (bounds == null) {
            previewGeofencePolygon?.let { removeOverlay(it) }
            previewGeofencePolygon = null
            requestInvalidate()
            return
        }
        val polygon =
            previewGeofencePolygon ?: Polygon().apply {
                map.overlays.add(this)
                previewGeofencePolygon = this
            }
        applyGeofenceStyle(polygon, preview = true)
        polygon.setPoints(geofencePoints(bounds))
        requestInvalidate()
    }

    fun syncCoverage(geofence: GeofenceBounds?, unvisited: List<GridCell>) {
        if (geofence == null) {
            clearCoverage()
            requestInvalidate()
            return
        }
        val desired = unvisited.associateBy { "${it.row}_${it.col}" }
        val stale = coveragePolygons.keys - desired.keys
        for (key in stale) {
            coveragePolygons.remove(key)?.let { removeOverlay(it) }
        }
        for ((key, cell) in desired) {
            val polygon =
                coveragePolygons.getOrPut(key) {
                    Polygon().apply {
                        fillColor = PackColor.argb(PackColor.MapCoverageFill)
                        outlinePaint.color = PackColor.argb(PackColor.MapAlert)
                        outlinePaint.strokeWidth = 1f
                        map.overlays.add(this)
                    }
                }
            polygon.setPoints(coverageCellPoints(geofence, cell))
        }
        requestInvalidate()
    }

    fun syncClusters(
        clusters: List<ApCluster>,
        onClusterClick: (ApCluster) -> Unit,
    ) {
        clusterClickHandler = onClusterClick
        val desired = clusters.associateBy { it.id }
        val stale = clusterMarkers.keys - desired.keys
        for (id in stale) {
            clusterMarkers.remove(id)?.let { removeOverlay(it) }
        }
        val context = map.context
        for ((id, cluster) in desired) {
            val marker =
                clusterMarkers.getOrPut(id) {
                    Marker(map).apply {
                        setAnchor(Marker.ANCHOR_CENTER, Marker.ANCHOR_CENTER)
                        setOnMarkerClickListener(clusterClickListener)
                        map.overlays.add(this)
                    }
                }
            setMarkerPosition(marker, cluster.lat, cluster.lon)
            marker.relatedObject = cluster
            val title =
                if (cluster.count > 1) {
                    "${cluster.count} devices"
                } else {
                    cluster.labelSsid
                }
            val snippet = "RSSI ${cluster.bestRssi} dBm"
            if (marker.title != title) marker.title = title
            if (marker.snippet != snippet) marker.snippet = snippet
            val icon = MapMarkerFactory.apIcon(context, cluster.signalKind)
            if (marker.icon !== icon) marker.icon = icon
        }
        requestInvalidate()
    }

    fun syncAlerts(
        pins: List<AlertPin>,
        onAlertClick: (AlertPin) -> Unit,
    ) {
        alertClickHandler = onAlertClick
        val desired = pins.associateBy { it.id }
        val stale = alertMarkers.keys - desired.keys
        for (id in stale) {
            alertMarkers.remove(id)?.let { removeOverlay(it) }
        }
        val context = map.context
        for ((id, pin) in desired) {
            val marker =
                alertMarkers.getOrPut(id) {
                    Marker(map).apply {
                        setAnchor(Marker.ANCHOR_CENTER, Marker.ANCHOR_CENTER)
                        infoWindow = null
                        setOnMarkerClickListener(alertClickListener)
                        map.overlays.add(this)
                    }
                }
            setMarkerPosition(marker, pin.lat, pin.lon)
            marker.relatedObject = pin
            if (marker.title != pin.title) marker.title = pin.title
            marker.snippet = null
            marker.infoWindow = null
            val icon = MapMarkerFactory.alertIcon(context, pin.kind)
            if (marker.icon !== icon) marker.icon = icon
        }
        requestInvalidate()
    }

    fun syncOperatorPin(point: GeoPoint?) {
        if (point == null) {
            operatorMarker?.let { removeOverlay(it) }
            operatorMarker = null
            requestInvalidate()
            return
        }
        val context = map.context
        val marker =
            operatorMarker ?: Marker(map).apply {
                setAnchor(Marker.ANCHOR_CENTER, Marker.ANCHOR_CENTER)
                infoWindow = null
                title = "Operator"
                map.overlays.add(this)
                operatorMarker = this
            }
        setMarkerPosition(marker, point.latitude, point.longitude)
        val icon = MapMarkerFactory.operatorIcon(context)
        if (marker.icon !== icon) marker.icon = icon
        requestInvalidate()
    }

    private fun setMarkerPosition(marker: Marker, lat: Double, lon: Double) {
        val current = marker.position
        if (current.latitude == lat && current.longitude == lon) return
        marker.position = GeoPoint(lat, lon)
    }

    private fun requestInvalidate() {
        if (invalidatePending) return
        invalidatePending = true
        mainHandler.post {
            invalidatePending = false
            map.invalidate()
        }
    }

    private fun clearCoverage() {
        for (polygon in coveragePolygons.values) {
            removeOverlay(polygon)
        }
        coveragePolygons.clear()
    }

    private fun removeOverlay(overlay: Overlay) {
        map.overlays.remove(overlay)
    }

    private fun geofencePoints(bounds: GeofenceBounds): List<GeoPoint> =
        listOf(
            GeoPoint(bounds.latMin, bounds.lonMin),
            GeoPoint(bounds.latMin, bounds.lonMax),
            GeoPoint(bounds.latMax, bounds.lonMax),
            GeoPoint(bounds.latMax, bounds.lonMin),
            GeoPoint(bounds.latMin, bounds.lonMin),
        )

    private fun applyGeofenceStyle(polygon: Polygon, preview: Boolean) {
        polygon.fillColor =
            if (preview) {
                PackColor.argb(PackColor.MapGeofencePreviewFill)
            } else {
                PackColor.argb(PackColor.MapGeofenceFill)
            }
        polygon.outlinePaint.color =
            if (preview) {
                PackColor.argb(PackColor.Sun)
            } else {
                PackColor.argb(PackColor.Good)
            }
        polygon.outlinePaint.strokeWidth = 4f
    }

    private fun coverageCellPoints(geofence: GeofenceBounds, cell: GridCell): List<GeoPoint> {
        val rows = MapViewModel.COVERAGE_ROWS
        val cols = MapViewModel.COVERAGE_COLS
        val latSpan = geofence.latMax - geofence.latMin
        val lonSpan = geofence.lonMax - geofence.lonMin
        val latStep = latSpan / rows
        val lonStep = lonSpan / cols
        val lat0 = geofence.latMin + cell.row * latStep
        val lat1 = lat0 + latStep
        val lon0 = geofence.lonMin + cell.col * lonStep
        val lon1 = lon0 + lonStep
        return listOf(
            GeoPoint(lat0, lon0),
            GeoPoint(lat0, lon1),
            GeoPoint(lat1, lon1),
            GeoPoint(lat1, lon0),
            GeoPoint(lat0, lon0),
        )
    }
}
