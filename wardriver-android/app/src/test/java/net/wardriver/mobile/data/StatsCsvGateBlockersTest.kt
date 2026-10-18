package net.wardriver.mobile.data

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** CSV gate blocker labels and home-zone predicate for dashboard glance card. */
class StatsCsvGateBlockersTest {
    @Test
    fun untrustedClockShowsPhoneTimeUnset() {
        val blockers =
            CsvGateHelpers.csvGateBlockers(
                scanAllowed = true,
                clockOk = false,
                gpsFixOk = true,
                insideHome = false,
            )
        assertTrue(blockers.contains("Phone time unset"))
    }

    @Test
    fun sessionActiveUsesLatchNotPointInCircle() {
        assertTrue(
            CsvGateHelpers.csvInsideHome(
                sessionActive = true,
                latchSuppressed = true,
                gpsFixOk = false,
                pointInCircle = false,
            ),
        )
        assertFalse(
            CsvGateHelpers.csvInsideHome(
                sessionActive = true,
                latchSuppressed = false,
                gpsFixOk = true,
                pointInCircle = true,
            ),
        )
    }

    @Test
    fun idleRequiresGpsOkForInsideHome() {
        assertFalse(
            CsvGateHelpers.csvInsideHome(
                sessionActive = false,
                latchSuppressed = false,
                gpsFixOk = false,
                pointInCircle = true,
            ),
        )
        assertTrue(
            CsvGateHelpers.csvInsideHome(
                sessionActive = false,
                latchSuppressed = false,
                gpsFixOk = true,
                pointInCircle = true,
            ),
        )
    }
}
