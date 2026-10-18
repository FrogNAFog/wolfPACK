package net.wardriver.mobile.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FleetRepositoryTest {
    @Test
    fun nextFreeFleetSlot_returnsFirstFreeSlot() {
        assertEquals(1, FleetRepository.nextFreeFleetSlot(emptyList()))
        assertEquals(2, FleetRepository.nextFreeFleetSlot(listOf(1)))
        assertEquals(5, FleetRepository.nextFreeFleetSlot(listOf(1, 2, 3, 4)))
    }

    @Test
    fun nextFreeFleetSlot_returnsNullWhenAllInUse() {
        val all = (1..FleetRepository.FLEET_MAX_SNIFFERS).toList()
        assertNull(FleetRepository.nextFreeFleetSlot(all))
    }

    @Test
    fun nextFreeFleetSlot_offersSlot48When47InUse() {
        // 47 slots (1..47) in use; with the inclusive upper bound slot 48 is adoptable.
        val used = (1..47).toList()
        assertEquals(48, FleetRepository.nextFreeFleetSlot(used))
    }
}
