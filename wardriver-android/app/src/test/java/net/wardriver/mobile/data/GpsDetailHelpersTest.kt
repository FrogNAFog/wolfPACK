package net.wardriver.mobile.data

import net.wardriver.mobile.ingest.GpsFreshness
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class GpsDetailHelpersTest {
    @Test
    fun formatDetailFreshOmitsStaleSuffix() {
        assertEquals(
            "8 m acc - 120 m alt",
            GpsDetailHelpers.formatDetail(8f, 120.0, fixAgeMs = 2_000L),
        )
    }

    @Test
    fun formatDetailStaleAppendsAgeSeconds() {
        assertEquals(
            "87 m acc - stale 12s",
            GpsDetailHelpers.formatDetail(87f, null, fixAgeMs = 12_500L),
        )
    }

    @Test
    fun formatDetailNoAccuracyReturnsNoFix() {
        assertEquals("No fix", GpsDetailHelpers.formatDetail(null, null, fixAgeMs = 5_000L))
    }

    @Test
    fun formatDetailStaleAtExactlyMaxFreshAge() {
        val detail =
            GpsDetailHelpers.formatDetail(
                10f,
                null,
                fixAgeMs = GpsFreshness.MAX_FIX_AGE_MS,
            )
        assertFalse(detail.contains("stale"))
    }

    @Test
    fun resolveDisplayLocationIdleUsesIdleFix() {
        val idle = Any()
        assertSame(
            idle,
            GpsDetailHelpers.resolveDisplayLocation(
                sessionActive = false,
                wardrive = null,
                idle = idle,
            ),
        )
    }

    @Test
    fun resolveDisplayLocationActiveIgnoresIdleWhenWardriveNull() {
        val idle = Any()
        assertNull(
            GpsDetailHelpers.resolveDisplayLocation(
                sessionActive = true,
                wardrive = null,
                idle = idle,
            ),
        )
    }

    @Test
    fun resolveDisplayLocationActivePrefersWardrive() {
        val wardrive = Any()
        val idle = Any()
        assertSame(
            wardrive,
            GpsDetailHelpers.resolveDisplayLocation(
                sessionActive = true,
                wardrive = wardrive,
                idle = idle,
            ),
        )
    }

    @Test
    fun waitingForGpsIsAsciiLabel() {
        assertEquals("Waiting for GPS", GpsDetailHelpers.WAITING_FOR_GPS)
    }
}
