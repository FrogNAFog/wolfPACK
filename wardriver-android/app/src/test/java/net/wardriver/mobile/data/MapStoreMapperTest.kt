package net.wardriver.mobile.data

import net.wardriver.mobile.db.NetworkSummaryEntity
import net.wardriver.mobile.db.ProbeObservationEntity
import net.wardriver.mobile.db.RoutePointEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionMapMarkerEntity
import net.wardriver.mobile.ui.map.GeofenceBounds
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Unit tests for MapStore entity -> UI mappers. */
class MapStoreMapperTest {
    @Test
    fun summaryMapsToApMarker() {
        val summary =
            NetworkSummaryEntity(
                bssid = "aa:bb:cc:dd:ee:01",
                ssid = "MyNet",
                firstLat = 37.0,
                firstLon = -122.0,
                firstSeenUnixMs = 1_000L,
                lastRssi = -70,
                lastLat = 37.01,
                lastLon = -122.01,
                lastSeenUnixMs = 2_000L,
                observationCount = 5,
                lastSignalKind = "WIFI",
            )
        val marker = summary.toApMarkerFromSummary()
        assertEquals(37.0, marker.lat, 0.0)
        assertEquals(-122.0, marker.lon, 0.0)
        assertEquals("MyNet", marker.ssid)
        assertEquals(-70, marker.rssi)
        assertEquals("aa:bb:cc:dd:ee:01", marker.bssid)
        assertEquals("WIFI", marker.signalKind)
    }

    @Test
    fun sessionMarkerMapsToApMarker() {
        val marker =
            SessionMapMarkerEntity(
                sessionId = 1L,
                bssid = "aa:bb:cc:dd:ee:02",
                lat = 40.5,
                lon = -75.5,
                ssid = "HomeNet",
                rssi = -60,
                signalKind = "BLE",
                observedUnixMs = 3_000L,
            )
        val ap = marker.toApMarker()
        assertEquals(40.5, ap.lat, 0.0)
        assertEquals(-75.5, ap.lon, 0.0)
        assertEquals("HomeNet", ap.ssid)
        assertEquals(-60, ap.rssi)
        assertEquals("BLE", ap.signalKind)
    }

    @Test
    fun probeObservationMapsToProbeApMarker() {
        val probe =
            ProbeObservationEntity(
                sessionId = 1L,
                snifferId = 2,
                observedUnixMs = 4_000L,
                lat = 41.0,
                lon = -76.0,
                altitudeM = 10,
                accuracyM = 9f,
                srcMac = "11:22:33:44:55:66",
                ssid = "",
                channel = 6,
                rssi = -85,
            )
        val ap = probe.toApMarker()
        assertEquals(41.0, ap.lat, 0.0)
        assertEquals(-76.0, ap.lon, 0.0)
        assertEquals("11:22:33:44:55:66", ap.bssid)
        assertEquals("PROBE_REQ", ap.signalKind)
    }

    @Test
    fun routePointEntityMapsToRoutePoint() {
        val entity =
            RoutePointEntity(
                id = 7L,
                sessionId = 1L,
                unixMs = 5_000L,
                lat = 38.1,
                lon = -121.2,
                accuracyM = 9f,
            )
        val point = entity.toRoutePoint()
        assertEquals(7L, point.id)
        assertEquals(38.1, point.lat, 0.0)
        assertEquals(-121.2, point.lon, 0.0)
    }

    @Test
    fun sessionWithGeofenceMapsToBounds() {
        val session =
            SessionEntity(
                startedUnixMs = 1_000L,
                endedUnixMs = 2_000L,
                gatewayMac = "aa:bb:cc:dd:ee:03",
                geofenceLatMin = 37.0,
                geofenceLatMax = 38.0,
                geofenceLonMin = -123.0,
                geofenceLonMax = -122.0,
            )
        val bounds = session.toGeofenceBounds()
        assertEquals(
            GeofenceBounds(37.0, 38.0, -123.0, -122.0),
            bounds,
        )
    }

    @Test
    fun sessionWithoutGeofenceReturnsNull() {
        val session =
            SessionEntity(
                startedUnixMs = 1_000L,
                endedUnixMs = 2_000L,
                gatewayMac = "aa:bb:cc:dd:ee:03",
                geofenceLatMin = null,
                geofenceLatMax = null,
                geofenceLonMin = null,
                geofenceLonMax = null,
            )
        assertNull(session.toGeofenceBounds())
    }

    @Test
    fun sessionWithDegenerateGeofenceReturnsNull() {
        val session =
            SessionEntity(
                startedUnixMs = 1_000L,
                endedUnixMs = 2_000L,
                gatewayMac = "aa:bb:cc:dd:ee:03",
                geofenceLatMin = 38.0,
                geofenceLatMax = 37.0,
                geofenceLonMin = -123.0,
                geofenceLonMax = -122.0,
            )
        assertNull(session.toGeofenceBounds())
    }
}
