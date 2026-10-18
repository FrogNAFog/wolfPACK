package net.wardriver.mobile.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LineListValidatorsTest {
    @Test
    fun validateSsidLines_accepts_normal_ssids() {
        assertNull(LineListValidators.validateSsidLines("MyNetwork\nGuest_WiFi"))
    }

    @Test
    fun validateSsidLines_rejects_long_ssid() {
        val longSsid = "a".repeat(33)
        val err = LineListValidators.validateSsidLines(longSsid)
        assertNotNull(err)
        assertEquals(1, err!!.lineNumber)
    }

    @Test
    fun validateSsidLines_rejects_too_many_unique_lines() {
        val lines = (1..33).joinToString("\n") { "ssid-$it" }
        val err = LineListValidators.validateSsidLines(lines)
        assertNotNull(err)
    }

    @Test
    fun validateSsidLinesMessage_reports_byte_limit() {
        val longSsid = "x".repeat(33)
        val msg = LineListValidators.validateSsidLinesMessage(longSsid)
        assertNotNull(msg)
        assertTrue(msg!!.contains("32 bytes"))
    }

    @Test
    fun normalizeSsidLines_trims_dedupes_and_drops_blanks() {
        assertEquals(
            "foo\nbar",
            LineListValidators.normalizeSsidLines(" foo \n\nbar\nfoo\n"),
        )
    }
}
