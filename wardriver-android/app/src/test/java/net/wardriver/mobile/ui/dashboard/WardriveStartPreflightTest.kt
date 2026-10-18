package net.wardriver.mobile.ui.dashboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class WardriveStartPreflightTest {
    @Test
    fun clockBlocksStartWhenUntrusted() {
        assertEquals(
            WardriveStartPreflight.PHONE_TIME_UNSET_MESSAGE,
            WardriveStartPreflight.clockBlocksStart(clockOk = false),
        )
    }

    @Test
    fun clockDoesNotBlockWhenTrusted() {
        assertNull(WardriveStartPreflight.clockBlocksStart(clockOk = true))
    }
}
