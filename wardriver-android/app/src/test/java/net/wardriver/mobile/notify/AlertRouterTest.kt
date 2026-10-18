package net.wardriver.mobile.notify

import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.ingest.EventNotifyAlert
import org.junit.Assert.assertEquals
import org.junit.Test

/** Unit tests for the merged AlertRouter title helper and Detection/RemoteId config. */
class AlertRouterTest {
    private fun alert(
        kind: String,
        detectionMethodId: Int? = null,
    ) =
        EventNotifyAlert(
            kind = kind,
            snifferId = 1,
            unixMs = 1_000L,
            detail = "mac=aa:bb:cc:dd:ee:01",
            lat = 37.0,
            lon = -122.0,
            detectionMethodId = detectionMethodId,
        )

    @Test
    fun axonMethodWinsOverKind() {
        assertEquals("Axon alert", methodBasedTitle(alert(AlertKind.DETECTION_WIFI, 4)))
        assertEquals("Axon alert", methodBasedTitle(alert(AlertKind.REMOTE_ID, 35)))
    }

    @Test
    fun iProMethodWinsOverKind() {
        assertEquals("i-PRO alert", methodBasedTitle(alert(AlertKind.DETECTION_BLE, 5)))
        assertEquals("i-PRO alert", methodBasedTitle(alert(AlertKind.FLOCK_WIFI, 36)))
    }

    @Test
    fun wifiKindGetsCameraTitle() {
        assertEquals("Camera WiFi alert", methodBasedTitle(alert(AlertKind.DETECTION_WIFI)))
        assertEquals("Camera WiFi alert", methodBasedTitle(alert(AlertKind.FLOCK_WIFI)))
    }

    @Test
    fun bleKindGetsDetectionTitle() {
        assertEquals("Detection alert", methodBasedTitle(alert(AlertKind.DETECTION_BLE)))
        assertEquals("Detection alert", methodBasedTitle(alert(AlertKind.FLOCK_BLE)))
    }

    @Test
    fun remoteIdKindGetsRemoteIdTitle() {
        assertEquals("Remote ID alert", methodBasedTitle(alert(AlertKind.REMOTE_ID)))
    }

    @Test
    fun unknownKindFallsBackToDetectionTitle() {
        assertEquals("Detection alert", methodBasedTitle(alert("SOMETHING_ELSE")))
    }

    @Test
    fun detectionAndRemoteIdUseDistinctNotificationOffsets() {
        // RemoteIdAlertRouter adds a fixed offset; DetectionAlertRouter does not.
        // Assert the constants are wired so the two families never collide in the shade.
        assertEquals(2000, AlertChannels.REMOTE_ID_NOTIFICATION_OFFSET)
        assertEquals(1000, AlertChannels.DETECTION_NOTIFICATION_BASE_ID)
    }
}
