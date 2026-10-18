package net.wardriver.mobile.ingest

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class WardriveClockTest {
    private val floor = 1_700_000_000_000L
    private val ceiling = floor + 10L * 365 * 24 * 60 * 60 * 1000

    @Test
    fun isPlausibleRejectsBeforeFloor() {
        assertFalse(WardriveClock.isPlausible(floor - 1, floor, ceiling))
    }

    @Test
    fun isPlausibleAcceptsFloorInclusive() {
        assertTrue(WardriveClock.isPlausible(floor, floor, ceiling))
    }

    @Test
    fun isPlausibleAcceptsMidWindow() {
        assertTrue(WardriveClock.isPlausible(floor + 1_000_000L, floor, ceiling))
    }

    @Test
    fun isPlausibleRejectsAtCeiling() {
        assertFalse(WardriveClock.isPlausible(ceiling, floor, ceiling))
    }

    @Test
    fun isPlausibleRejectsAfterCeiling() {
        assertFalse(WardriveClock.isPlausible(ceiling + 1, floor, ceiling))
    }
}
