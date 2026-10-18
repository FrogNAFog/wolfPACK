package net.wardriver.mobile.ui.map

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MapViewModelSessionTest {
    @Test
    fun resolveDisplaySessionId_prefersActiveSession() {
        assertEquals(42L, MapViewModel.resolveDisplaySessionId(42L, 99L))
    }

    @Test
    fun resolveDisplaySessionId_fallsBackToLatestWhenIdle() {
        assertEquals(99L, MapViewModel.resolveDisplaySessionId(null, 99L))
    }

    @Test
    fun resolveDisplaySessionId_nullWhenNoSessions() {
        assertNull(MapViewModel.resolveDisplaySessionId(null, null))
    }
}
