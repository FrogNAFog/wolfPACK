package net.wardriver.mobile.ui.map

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MapGeometryTest {
    @Test
    fun filterViewport_keepsInsideExpandedBox() {
        val aps =
            listOf(
                ApMarker(37.0, -122.0, "in", -50, "aa:bb:cc:dd:ee:01", "WIFI"),
                ApMarker(38.0, -121.0, "out", -60, "aa:bb:cc:dd:ee:02", "WIFI"),
                ApMarker(0.0, 0.0, "invalid", -70, "aa:bb:cc:dd:ee:03", "WIFI"),
            )
        val filtered =
            MapGeometry.filterViewport(
                aps,
                south = 36.9,
                north = 37.1,
                west = -122.1,
                east = -121.9,
            )
        assertEquals(1, filtered.size)
        assertEquals("in", filtered[0].ssid)
    }

    @Test
    fun apCluster_stableId_usesBssidForSingleton() {
        val id = ApCluster.stableId(37.0, -122.0, 1, "aa:bb:cc:dd:ee:ff")
        assertEquals("aa:bb:cc:dd:ee:ff", id)
    }

    @Test
    fun apCluster_stableId_usesLatLonForCluster() {
        val id = ApCluster.stableId(37.123456, -122.654321, 3, "")
        assertTrue(id.contains("37.12346"))
        assertTrue(id.endsWith("_3"))
    }
}
