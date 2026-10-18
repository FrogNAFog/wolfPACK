package net.wardriver.mobile.notify

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AlertThrottleTest {
    @Test
    fun parseMac_extractsLowercaseMac() {
        val mac = AlertThrottle.parseMac("mac=AA:BB:CC:DD:EE:FF ch=6 rssi_avg=-50.0")
        assertEquals("aa:bb:cc:dd:ee:ff", mac)
    }

    @Test
    fun parseMac_readsJsonMacField() {
        val detail = """{"mac":"AA:BB:CC:DD:EE:FF","channel":6,"rssi":-55,"method":64}"""
        assertEquals("aa:bb:cc:dd:ee:ff", AlertThrottle.parseMac(detail))
    }

    @Test
    fun shouldNotify_suppressesWithinCooldown() {
        val key = "FLOCK_WIFI:aa:bb:cc:dd:ee:01"
        val first = AlertThrottle.shouldNotify(key, nowMs = 1_000, cooldownMs = 30_000)
        assertTrue(first is NotifyDecision.Post)
        assertEquals(0, (first as NotifyDecision.Post).suppressedCount)

        val second = AlertThrottle.shouldNotify(key, nowMs = 10_000, cooldownMs = 30_000)
        assertTrue(second is NotifyDecision.Suppress)

        val third = AlertThrottle.shouldNotify(key, nowMs = 35_000, cooldownMs = 30_000)
        assertTrue(third is NotifyDecision.Post)
        assertEquals(1, (third as NotifyDecision.Post).suppressedCount)
    }

    @Test
    fun stableNotificationId_isStablePerMac() {
        val mac = "aa:bb:cc:dd:ee:ff"
        assertEquals(
            AlertThrottle.stableNotificationId(mac),
            AlertThrottle.stableNotificationId(mac),
        )
    }
}
