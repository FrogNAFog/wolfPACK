package net.wardriver.mobile.notify

import org.junit.Assert.assertEquals
import org.junit.Test

class GeofenceAlertBusTest {
    @Test
    fun enterCopy_matchesPlan() {
        val (title, subtitle) =
            GeofenceTransitionNotifier.Transition.Enter.let { transition ->
                when (transition) {
                    GeofenceTransitionNotifier.Transition.Enter ->
                        "Inside wardrive square" to "Wardrive geofence active"
                    GeofenceTransitionNotifier.Transition.Leave ->
                        "Left wardrive square" to "Return to your geofence to continue wardriving"
                }
            }
        assertEquals("Inside wardrive square", title)
        assertEquals("Wardrive geofence active", subtitle)
    }

    @Test
    fun leaveCopy_matchesPlan() {
        val (title, subtitle) =
            when (GeofenceTransitionNotifier.Transition.Leave) {
                GeofenceTransitionNotifier.Transition.Enter ->
                    "Inside wardrive square" to "Wardrive geofence active"
                GeofenceTransitionNotifier.Transition.Leave ->
                    "Left wardrive square" to "Return to your geofence to continue wardriving"
            }
        assertEquals("Left wardrive square", title)
        assertEquals("Return to your geofence to continue wardriving", subtitle)
    }
}
