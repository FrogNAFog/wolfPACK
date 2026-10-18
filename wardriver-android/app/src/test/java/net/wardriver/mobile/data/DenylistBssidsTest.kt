package net.wardriver.mobile.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DenylistBssidsTest {
    @Test
    fun validateMacLines_accepts_colon_format() {
        assertNull(
            DenylistBssids.validateMacLines(
                "AA:BB:CC:DD:EE:FF\n11:22:33:44:55:66",
            ),
        )
    }

    @Test
    fun validateMacLines_rejects_compact_hex() {
        val err = DenylistBssids.validateMacLines("AABBCCDDEEFF")
        assertNotNull(err)
        assertEquals(1, err!!.lineNumber)
    }

    @Test
    fun validateMacLines_rejects_dashes() {
        val err = DenylistBssids.validateMacLines("AA-BB-CC-DD-EE-FF")
        assertNotNull(err)
        assertEquals(1, err!!.lineNumber)
    }

    @Test
    fun validateMacLines_reports_line_number() {
        val err =
            DenylistBssids.validateMacLines(
                "AA:BB:CC:DD:EE:FF\nnot-a-mac",
            )
        assertNotNull(err)
        assertEquals(2, err!!.lineNumber)
        assertTrue(err.line.contains("not-a-mac"))
    }

    @Test
    fun normalizeMacLines_uppercases_and_drops_blanks() {
        assertEquals(
            "AA:BB:CC:DD:EE:FF\n11:22:33:44:55:66",
            DenylistBssids.normalizeMacLines("aa:bb:cc:dd:ee:ff\n\n11:22:33:44:55:66\n"),
        )
    }

    @Test
    fun mergedManualPlusGateway_prepends_gateway_and_dedupes() {
        val merged =
            DenylistBssids.mergedManualPlusGateway(
                manualLines = listOf("AA:BB:CC:DD:EE:FF", "11:22:33:44:55:66"),
                gatewayMac = "aa:bb:cc:dd:ee:ff",
            )
        assertEquals(
            listOf("AA:BB:CC:DD:EE:FF", "11:22:33:44:55:66"),
            merged,
        )
    }

    @Test
    fun mergedManualPlusGateway_gateway_only_when_manual_empty() {
        assertEquals(
            listOf("DE:AD:BE:EF:00:01"),
            DenylistBssids.mergedManualPlusGateway(
                manualLines = emptyList(),
                gatewayMac = "DE:AD:BE:EF:00:01",
            ),
        )
    }

    @Test
    fun formatValidationMessage_includes_line_number() {
        val msg =
            DenylistBssids.formatValidationMessage(
                LineListValidators.ValidationError(3, "bad", "test error"),
            )
        assertTrue(msg.contains("Line 3"))
        assertTrue(msg.contains("AA:BB:CC:DD:EE:FF"))
    }
}
