package net.wardriver.mobile.ui.map

import net.wardriver.mobile.NativeCore
import org.osmdroid.util.GeoPoint

/** Douglas-Peucker polyline simplification and AP clustering via Rust [`map_geometry`]. */
object MapGeometry {
    const val ROUTE_SIMPLIFY_THRESHOLD = 5000
    const val CLUSTER_ZOOM_THRESHOLD = 14.0
    const val COORD_DECIMALS = 5

    fun simplifyRoute(points: List<GeoPoint>, maxPoints: Int = ROUTE_SIMPLIFY_THRESHOLD): List<GeoPoint> {
        if (points.size <= maxPoints) return points
        val lats = DoubleArray(points.size)
        val lons = DoubleArray(points.size)
        for (i in points.indices) {
            lats[i] = points[i].latitude
            lons[i] = points[i].longitude
        }
        val native = NativeCore.simplifyRouteFromArrays(lats, lons, maxPoints)
        if (native != null) {
            val (outLats, outLons) = native
            return List(outLats.size) { i -> GeoPoint(outLats[i], outLons[i]) }
        }
        return points.take(maxPoints)
    }

    const val VIEWPORT_MARGIN_FRACTION = 0.2

    /** Keep AP markers inside expanded viewport; skip invalid (0,0) fixes. */
    fun filterViewport(
        aps: List<ApMarker>,
        south: Double,
        north: Double,
        west: Double,
        east: Double,
        marginFraction: Double = VIEWPORT_MARGIN_FRACTION,
    ): List<ApMarker> {
        if (aps.isEmpty()) return aps
        val latSpan = north - south
        val lonSpan = east - west
        val latMargin = latSpan * marginFraction
        val lonMargin = lonSpan * marginFraction
        val s = south - latMargin
        val n = north + latMargin
        val w = west - lonMargin
        val e = east + lonMargin
        return aps.filter { ap ->
            if (ap.lat == 0.0 && ap.lon == 0.0) return@filter false
            ap.lat in s..n && ap.lon in w..e
        }
    }

    fun clusterMapMarkers(
        aps: List<ApMarker>,
        zoom: Double,
        clusterZoomThreshold: Double = CLUSTER_ZOOM_THRESHOLD,
        coordDecimals: Int = COORD_DECIMALS,
    ): List<ApCluster> {
        if (aps.isEmpty()) return emptyList()
        val lats = DoubleArray(aps.size)
        val lons = DoubleArray(aps.size)
        val rssis = IntArray(aps.size)
        val ssids = Array(aps.size) { "" }
        val bssids = Array(aps.size) { "" }
        val kinds = Array(aps.size) { "WIFI" }
        for (i in aps.indices) {
            val ap = aps[i]
            lats[i] = ap.lat
            lons[i] = ap.lon
            rssis[i] = ap.rssi
            ssids[i] = ap.ssid
            bssids[i] = ap.bssid
            kinds[i] = ap.signalKind
        }
        val native =
            NativeCore.clusterMapMarkersFromArrays(
                lats = lats,
                lons = lons,
                rssis = rssis,
                ssids = ssids,
                bssids = bssids,
                signalKinds = kinds,
                zoom = zoom,
                clusterZoomThreshold = clusterZoomThreshold,
                coordDecimals = coordDecimals,
            )
        if (native == null) {
            return aps.map { ap ->
                ApCluster(
                    id = ApCluster.stableId(ap.lat, ap.lon, 1, ap.bssid),
                    lat = ap.lat,
                    lon = ap.lon,
                    count = 1,
                    labelSsid = ap.ssid,
                    bestRssi = ap.rssi,
                    signalKind = ap.signalKind,
                    members = listOf(ap),
                )
            }
        }
        val out = mutableListOf<ApCluster>()
        for (i in native.lats.indices) {
            val members =
                native.memberIndices[i].toList().mapNotNull { idx ->
                    aps.getOrNull(idx)
                }
            val signalKind =
                members.maxByOrNull { it.rssi }?.signalKind
                    ?: members.firstOrNull()?.signalKind
                    ?: "WIFI"
            val primaryBssid = members.firstOrNull()?.bssid.orEmpty()
            val count = native.counts[i]
            out.add(
                ApCluster(
                    id = ApCluster.stableId(native.lats[i], native.lons[i], count, primaryBssid),
                    lat = native.lats[i],
                    lon = native.lons[i],
                    count = count,
                    labelSsid = native.labelSsids[i],
                    bestRssi = native.bestRssi[i],
                    signalKind = signalKind,
                    members = members,
                ),
            )
        }
        return out
    }
}

data class ApMarker(
    val lat: Double,
    val lon: Double,
    val ssid: String,
    val rssi: Int,
    val bssid: String,
    val signalKind: String = "WIFI",
)

data class ApCluster(
    val id: String,
    val lat: Double,
    val lon: Double,
    val count: Int,
    val labelSsid: String,
    val bestRssi: Int,
    val signalKind: String = "WIFI",
    val members: List<ApMarker> = emptyList(),
) {
    companion object {
        fun stableId(lat: Double, lon: Double, count: Int, bssid: String): String =
            if (count == 1 && bssid.isNotEmpty()) {
                bssid
            } else {
                "${"%.5f".format(lat)}_${"%.5f".format(lon)}_$count"
            }
    }
}
