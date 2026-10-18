package net.wardriver.mobile.ui.map

import net.wardriver.mobile.db.DetectionEntity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MapAlertPinsTest {
    @Test
    fun detectionWithStoredCoords_createsPinWithoutRoute() {
        val det =
            DetectionEntity(
                sessionId = 1L,
                kind = "FLOCK_WIFI",
                unixMs = 1_000L,
                detail = "mac=aa:bb:cc:dd:ee:01 ch=6 rssi_avg=-50",
                lat = 40.0,
                lon = -75.0,
            )
        val pins = buildAlertPins(listOf(det))
        assertEquals(1, pins.size)
        assertEquals(40.0, pins[0].lat, 0.0)
        assertEquals(-75.0, pins[0].lon, 0.0)
    }

    @Test
    fun detectionWithoutCoordsOrRoute_dropsPin() {
        val det =
            DetectionEntity(
                sessionId = 1L,
                kind = "FLOCK_WIFI",
                unixMs = 1_000L,
                detail = "mac=aa:bb:cc:dd:ee:01 ch=6 rssi_avg=-50",
            )
        assertTrue(buildAlertPins(listOf(det)).isEmpty())
    }

    @Test
    fun geofenceDetail_parsesLegacyLatLon() {
        val det =
            DetectionEntity(
                sessionId = 1L,
                kind = "GEOFENCE_LEAVE",
                unixMs = 2_000L,
                detail = "lat=41.5,lon=-72.1",
            )
        val coords = detectionCoords(det)
        assertEquals(41.5, coords?.first)
        assertEquals(-72.1, coords?.second)
    }

    @Test
    fun storedCoordsTakePriorityOverDetailParse() {
        val det =
            DetectionEntity(
                sessionId = 1L,
                kind = "GEOFENCE_LEAVE",
                unixMs = 2_000L,
                detail = "lat=1.0,lon=2.0",
                lat = 9.0,
                lon = 8.0,
            )
        val coords = detectionCoords(det)
        assertEquals(9.0, coords?.first)
        assertEquals(8.0, coords?.second)
    }

    @Test
    fun detectionDetail_parsesDetectionMethodId() {
        val det =
            DetectionEntity(
                sessionId = 1L,
                kind = "FLOCK_WIFI",
                unixMs = 1_000L,
                detail = "mac=aa:bb:cc:dd:ee:01 ch=6 rssi_avg=-50 method=2",
                lat = 40.0,
                lon = -75.0,
                detectionMethodId = 2,
            )
        val pins = buildAlertPins(listOf(det))
        assertEquals(2, pins[0].detectionMethodId)
    }

    @Test
    fun alertTitle_includesDetectionId() {
        assertEquals("Detection alert · ID 2", alertTitle("FLOCK_WIFI", 2))
    }

    @Test
    fun alertTitle_axonAndIpro() {
        assertEquals("Axon", alertTitle("FLOCK_BLE", 35))
        assertEquals("Axon", alertTitle("FLOCK_WIFI", 4))
        assertEquals("i-PRO", alertTitle("FLOCK_BLE", 36))
        assertEquals("i-PRO", alertTitle("FLOCK_WIFI", 5))
    }

    @Test
    fun alertTitle_remoteId() {
        assertEquals("Remote ID", alertTitle("REMOTE_ID", 64))
    }

    @Test
    fun parseDetectionMethodId_readsMethodToken() {
        assertEquals(35, parseDetectionMethodId("mac=aa:bb:cc:dd:ee:01 company=0x004C rssi=-60 method=35"))
    }

    @Test
    fun parseDetectionMethodId_readsJsonMethod() {
        val detail = """{"mac":"aa:bb:cc:dd:ee:01","method":64,"channel":6,"rssi":-55}"""
        assertEquals(64, parseDetectionMethodId(detail))
    }

    @Test
    fun parseRemoteIdDetail_readsJsonFields() {
        val detail =
            """
            {
              "mac":"aa:bb:cc:dd:ee:01",
              "channel":6,
              "rssi":-55,
              "method":64,
              "uas_id":"TEST-UAS",
              "id_type":1,
              "ua_type":2,
              "status":2,
              "operator_lat":40.1,
              "operator_lon":-75.1,
              "operator_location_type":1
            }
            """.trimIndent()
        val rid = parseRemoteIdDetail(detail)
        assertEquals("aa:bb:cc:dd:ee:01", rid?.mac)
        assertEquals(64, rid?.method)
        assertEquals(1, rid?.idType)
        assertEquals(40.1, rid?.operatorLat)
        assertEquals(-75.1, rid?.operatorLon)
    }

    @Test
    fun remoteIdFocusedLayers_hideOthers() {
        val selected =
            AlertPin(
                id = "rid_1",
                lat = 40.0,
                lon = -75.0,
                kind = "REMOTE_ID",
                title = "Remote ID",
                detail = "{}",
                unixMs = 100L,
            )
        val other =
            AlertPin(
                id = "flock_1",
                lat = 41.0,
                lon = -76.0,
                kind = "FLOCK_WIFI",
                title = "Flock",
                detail = "x",
                unixMs = 101L,
            )
        val pins = listOf(selected, other)
        assertEquals(listOf(selected), remoteIdFocusedAlertPins(pins, selected))
        assertEquals(emptyList<ApCluster>(), remoteIdFocusedClusters(listOf(ApCluster("c", 1.0, 2.0, 1, "ap", -50, "WIFI")), selected))
    }

    @Test
    fun remoteIdOperatorPoint_rejectsZeroCoords() {
        val pin =
            AlertPin(
                id = "rid_1",
                lat = 40.0,
                lon = -75.0,
                kind = "REMOTE_ID",
                title = "Remote ID",
                detail = "{}",
                unixMs = 100L,
                remoteId =
                    RemoteIdDetail(
                        mac = "aa:bb:cc:dd:ee:01",
                        channel = 6,
                        rssi = -50,
                        method = 64,
                        operatorLat = 0.0,
                        operatorLon = 0.0,
                    ),
            )
        assertNull(remoteIdOperatorPoint(pin))
    }
}
