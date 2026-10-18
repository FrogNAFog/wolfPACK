package net.wardriver.mobile.upload

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class WigleQrParserTest {
    @Test
    fun parse_validQr_returnsApiNameAndToken() {
        val creds = parseWigleActivateQr("mylogin:MyApiName:abc123token")
        assertEquals(WigleQrCredentials("MyApiName", "abc123token"), creds)
    }

    @Test
    fun parse_trimsWhitespace() {
        val creds = parseWigleActivateQr("  user:ApiName:token99  ")
        assertEquals(WigleQrCredentials("ApiName", "token99"), creds)
    }

    @Test
    fun parse_rejectsExtraColons() {
        assertNull(parseWigleActivateQr("user:part:extra:token"))
    }

    @Test
    fun parse_rejectsInvalidCharacters() {
        assertNull(parseWigleActivateQr("user:Api-Name:token"))
        assertNull(parseWigleActivateQr("user:ApiName:tok-en"))
    }

    @Test
    fun parse_rejectsBlankToken() {
        assertNull(parseWigleActivateQr("user:ApiName:"))
        assertNull(parseWigleActivateQr("user::token"))
    }
}
