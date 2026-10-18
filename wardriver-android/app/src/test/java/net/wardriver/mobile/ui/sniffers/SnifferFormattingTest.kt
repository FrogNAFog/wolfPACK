package net.wardriver.mobile.ui.sniffers

import net.wardriver.mobile.data.FleetRepository
import net.wardriver.mobile.policy.snifferIdAsWireU32
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SnifferFormattingTest {
    @Test
    fun snifferIdAsWireU32_masks_high_bit_ids() {
        assertEquals(0x8000_0000L, snifferIdAsWireU32(0x8000_0000.toInt()))
        assertEquals(0xDEAD_BEEFL, snifferIdAsWireU32(0xDEAD_BEEF.toInt()))
        assertEquals(0xFFFF_FFFFL, snifferIdAsWireU32(-1))
        assertEquals(5L, snifferIdAsWireU32(5))
    }

    @Test
    fun pendingNearbyVisible_boundary_and_clock() {
        val lastSeen = 10_000L
        assertTrue(pendingNearbyVisible(29_999L, lastSeen, gatewayConnected = true))
        assertFalse(pendingNearbyVisible(30_000L, lastSeen, gatewayConnected = true))
        assertFalse(pendingNearbyVisible(29_999L, lastSeen, gatewayConnected = false))
        assertFalse(pendingNearbyVisible(0L, lastSeen, gatewayConnected = true))
        assertFalse(pendingNearbyVisible(30_000L, 0L, gatewayConnected = true))
    }

    @Test
    fun filterPendingNearby_keeps_stale_rows_sorted_fresh_first() {
        val rows =
            listOf(
                statusRow(id = 1, lastSeen = 50_000L),
                statusRow(id = 2, lastSeen = 49_000L),
                statusRow(id = 3, lastSeen = 40_000L),
                statusRow(id = 4, lastSeen = 20_000L),
            )
        val filtered =
            filterPendingNearby(
                nearby = rows,
                adoptedIds = setOf(1),
                pendingIds = setOf(2),
                gatewayNowMs = 50_000L,
                gatewayConnected = true,
            )
        assertEquals(listOf(3, 4), filtered.map { it.snifferId })
        assertTrue(pendingNearbyVisible(50_000L, 40_000L, gatewayConnected = true))
        assertTrue(pendingNearbyStale(50_000L, 20_000L, gatewayConnected = true))
    }

    private fun statusRow(id: Int, lastSeen: Long): FleetRepository.SnifferStatusRow =
        FleetRepository.SnifferStatusRow(
            snifferId = id,
            mac = "02:11:22:33:44:55",
            lastSeenMs = lastSeen,
            state = 0,
            roleWire = 0,
            cpuLoadPct = 0,
            heapAvailPct = 0,
            fleetSlot = 0,
            rebootCount = 0,
        )
}
