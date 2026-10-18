package net.wardriver.mobile.ui.dashboard

import net.wardriver.mobile.ui.theme.PackColor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MetricIncrementStalenessTest {
    @Test
    fun stalenessColor_atZero_isGood() {
        assertEquals(PackColor.Good, IncrementStalenessTracker.stalenessColor(0L))
    }

    @Test
    fun stalenessColor_atMidpoint_isSun() {
        val mid = IncrementStalenessTracker.RAMP_MS / 2
        assertEquals(PackColor.Sun, IncrementStalenessTracker.stalenessColor(mid))
    }

    @Test
    fun stalenessColor_atRampEnd_isErr() {
        assertEquals(
            PackColor.Err,
            IncrementStalenessTracker.stalenessColor(IncrementStalenessTracker.RAMP_MS),
        )
        assertEquals(
            PackColor.Err,
            IncrementStalenessTracker.stalenessColor(IncrementStalenessTracker.RAMP_MS + 5_000L),
        )
    }

    @Test
    fun noteSample_incrementResetsTimer() {
        val tracker = IncrementStalenessTracker()
        tracker.noteSample(StalenessMetric.ObservationRows, 10L, nowMs = 1_000L)
        tracker.noteSample(StalenessMetric.ObservationRows, 10L, nowMs = 20_000L)
        val stale = tracker.colorForMetric(StalenessMetric.ObservationRows, nowMs = 20_000L)
        assertNotNull(stale)
        // 19s into 30s ramp — past midpoint, toward Err (not still Good).
        assertTrue(stale!!.red > PackColor.Good.red || stale.green < PackColor.Good.green)

        tracker.noteSample(StalenessMetric.ObservationRows, 11L, nowMs = 20_500L)
        assertEquals(
            PackColor.Good,
            tracker.colorForMetric(StalenessMetric.ObservationRows, nowMs = 20_500L),
        )
    }

    @Test
    fun colorForMetric_nullBeforeSample() {
        val tracker = IncrementStalenessTracker()
        assertNull(tracker.colorForMetric(StalenessMetric.CellTowers, nowMs = 1L))
        assertFalse(tracker.hasSample(StalenessMetric.CellTowers))
    }

    @Test
    fun reset_clearsSamples_soUiMapsToText() {
        val tracker = IncrementStalenessTracker()
        tracker.noteSample(StalenessMetric.UniqueWifiMacs, 5L, nowMs = 0L)
        tracker.noteSample(StalenessMetric.UniqueWifiMacs, 5L, nowMs = 30_000L)
        assertEquals(
            PackColor.Err,
            tracker.colorForMetric(StalenessMetric.UniqueWifiMacs, nowMs = 30_000L),
        )

        tracker.reset()
        assertNull(tracker.colorForMetric(StalenessMetric.UniqueWifiMacs, nowMs = 30_000L))
        assertFalse(tracker.hasSample(StalenessMetric.UniqueWifiMacs))
    }
}
