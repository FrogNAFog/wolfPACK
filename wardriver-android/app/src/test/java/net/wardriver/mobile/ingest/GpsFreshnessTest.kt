package net.wardriver.mobile.ingest

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GpsFreshnessTest {
    @Test
    fun freshWithinThreeSeconds() {
        val now = 10_000_000_000L
        assertTrue(GpsFreshness.isFreshElapsedNanos(now - 1_000_000_000L, now))
        assertTrue(GpsFreshness.isFreshElapsedNanos(now - 3_000_000_000L, now))
    }

    @Test
    fun staleBeyondThreeSeconds() {
        val now = 10_000_000_000L
        assertFalse(GpsFreshness.isFreshElapsedNanos(now - 3_001_000_000L, now))
    }

    @Test
    fun zeroElapsedRealtimeIsNotFresh() {
        assertFalse(GpsFreshness.isFreshElapsedNanos(0L, 10_000_000_000L))
    }

    @Test
    fun negativeAgeIsNotFresh() {
        val now = 10_000_000_000L
        assertFalse(GpsFreshness.isFreshElapsedNanos(now + 1_000_000L, now))
    }
}
