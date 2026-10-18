package net.wardriver.mobile.ui.map

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteIdLabelsTest {
    @Test
    fun basicIdTypeLabels() {
        assertEquals("Serial Number", remoteIdBasicIdTypeLabel(1))
        assertEquals("CAA Registration ID", remoteIdBasicIdTypeLabel(2))
    }

    @Test
    fun detectionMethodLabels() {
        assertEquals("Beacon vendor IE", remoteIdDetectionMethodLabel(64))
        assertEquals("NAN action", remoteIdDetectionMethodLabel(65))
        assertEquals("Service Data 0xFFFA", remoteIdDetectionMethodLabel(96))
    }

    @Test
    fun locationStatusLabels() {
        assertEquals("Airborne", remoteIdLocationStatusLabel(2))
        assertEquals("Emergency", remoteIdLocationStatusLabel(3))
    }

    @Test
    fun isPlausibleWallUnixMs_rejectsBootRelative() {
        val now = 1_720_000_000_000L
        assertFalse(isPlausibleWallUnixMs(45_000L, nowMs = now))
        assertFalse(isPlausibleWallUnixMs(0L, nowMs = now))
    }

    @Test
    fun isPlausibleWallUnixMs_acceptsRecentEpoch() {
        val now = 1_720_000_000_000L
        assertTrue(isPlausibleWallUnixMs(now - 5_000L, nowMs = now))
    }

    @Test
    fun formatRemoteIdDetectionTime_bootRelativeShowsNotSynced() {
        assertEquals("Time not synced", formatRemoteIdDetectionTime(12_345L))
    }

    @Test
    fun formatRemoteIdDetectionTime_formatsPlausibleEpoch() {
        val formatted = formatRemoteIdDetectionTime(1_700_000_000_000L)
        assertFalse(formatted == "Time not synced")
        assertTrue(formatted.contains("2023") || formatted.contains("2024"))
    }

    @Test
    fun remoteIdPanelBottomPaddingPx_isFortyPercent() {
        assertEquals(400, remoteIdPanelBottomPaddingPx(1000))
    }
}
