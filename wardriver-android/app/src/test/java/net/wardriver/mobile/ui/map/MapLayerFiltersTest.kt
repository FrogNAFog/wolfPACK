package net.wardriver.mobile.ui.map

import net.wardriver.mobile.data.ApMarkerKind
import org.junit.Assert.assertEquals
import org.junit.Test

class MapLayerFiltersTest {
    private val markers =
        listOf(
            ApMarker(1.0, 2.0, "wifi-ap", -50, "aa:01", "WIFI"),
            ApMarker(1.1, 2.1, "ble-dev", -60, "aa:02", "BLE"),
            ApMarker(1.2, 2.2, "cell", -70, "310260", "LTE"),
            ApMarker(1.3, 2.3, "probe", -80, "aa:03", ApMarkerKind.PROBE_REQ),
        )

    private val pins =
        listOf(
            AlertPin("1", 1.0, 2.0, "FLOCK_WIFI", "Flock", "detail", 100L),
            AlertPin("2", 1.0, 2.0, "FLOCK_BLE", "Flock", "detail", 101L),
            AlertPin("4", 1.0, 2.0, "SSID_WATCH", "SSID", "detail", 103L),
            AlertPin("5", 1.0, 2.0, "REMOTE_ID", "Remote ID", "detail", 104L),
        )

    @Test
    fun wifiOnly_hidesBleAndCell() {
        val filters = MapLayerFilters(wifi = true, ble = false, probe = false)
        val out = filters.filterApMarkers(markers)
        assertEquals(1, out.size)
        assertEquals("wifi-ap", out[0].ssid)
    }

    @Test
    fun bleOnly_hidesWifiAndCell() {
        val filters = MapLayerFilters(wifi = false, ble = true, probe = false)
        val out = filters.filterApMarkers(markers)
        assertEquals(1, out.size)
        assertEquals("ble-dev", out[0].ssid)
    }

    @Test
    fun probeOnly_hidesWifiAndBle() {
        val filters = MapLayerFilters(wifi = false, ble = false, probe = true)
        val out = filters.filterApMarkers(markers)
        assertEquals(1, out.size)
        assertEquals(ApMarkerKind.PROBE_REQ, out[0].signalKind)
    }

    @Test
    fun probeOff_hidesProbeMarkers() {
        val filters = MapLayerFilters(probe = false)
        val out = filters.filterApMarkers(markers)
        assertEquals(2, out.size)
        assertEquals(false, out.any { it.signalKind == ApMarkerKind.PROBE_REQ })
    }

    @Test
    fun remoteIdFilter_hidesRemoteIdPins() {
        val filters =
            MapLayerFilters(
                alerts = true,
                alertSsidWatch = true,
                alertRemoteId = false,
            )
        val out = filters.filterAlertPins(pins)
        assertEquals(3, out.size)
        assertEquals(false, out.any { it.kind == "REMOTE_ID" })
    }

    @Test
    fun detectionFilters_hideMatchingPins() {
        val filters =
            MapLayerFilters(
                alerts = false,
                alertSsidWatch = true,
                alertRemoteId = false,
            )
        val out = filters.filterAlertPins(pins)
        assertEquals(0, out.size)
    }

    @Test
    fun masterOff_hidesAllAlertPins() {
        val filters =
            MapLayerFilters(
                alerts = false,
                alertDetectionWifi = true,
                alertDetectionBle = true,
                alertSsidWatch = true,
                alertRemoteId = true,
                alertAxon = true,
                alertIPro = true,
            )
        val out = filters.filterAlertPins(pins)
        assertEquals(0, out.size)
    }

    @Test
    fun masterOn_withPerTypeOff_hidesOnlyThoseTypes() {
        val allPins = pins + listOf(
            AlertPin("6", 1.0, 2.0, "AXON", "Axon", "detail", 105L),
            AlertPin("7", 1.0, 2.0, "I-PRO", "i-PRO", "detail", 106L),
        )
        val filters =
            MapLayerFilters(
                alerts = true,
                alertDetectionWifi = false,
                alertDetectionBle = true,
                alertSsidWatch = false,
                alertRemoteId = true,
                alertAxon = true,
                alertIPro = false,
            )
        val out = filters.filterAlertPins(allPins)
        // FLOCK_WIFI off, SSID_WATCH off, I-PRO off => 3 remain (FLOCK_BLE, REMOTE_ID, AXON)
        assertEquals(3, out.size)
        assertEquals(false, out.any { it.kind == "FLOCK_WIFI" })
        assertEquals(false, out.any { it.kind == "SSID_WATCH" })
        assertEquals(false, out.any { it.kind == "I-PRO" })
        assertEquals(true, out.any { it.kind == "FLOCK_BLE" })
        assertEquals(true, out.any { it.kind == "REMOTE_ID" })
        assertEquals(true, out.any { it.kind == "AXON" })
    }

    @Test
    fun alertEnabledCount_countsCorrectly() {
        val filters =
            MapLayerFilters(
                alerts = true,
                alertDetectionWifi = true,
                alertDetectionBle = false,
                alertSsidWatch = true,
                alertRemoteId = false,
                alertAxon = true,
                alertIPro = true,
            )
        assertEquals(4, filters.alertEnabledCount)
    }
}
