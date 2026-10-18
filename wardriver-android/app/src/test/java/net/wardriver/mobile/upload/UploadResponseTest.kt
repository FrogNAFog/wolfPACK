package net.wardriver.mobile.upload

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UploadResponseTest {
    @Test
    fun wdgwars_okJson_isSuccess() {
        assertTrue(responseIndicatesSuccess("""{"ok":true,"rows":42}""", "ok"))
    }

    @Test
    fun wdgwars_okJsonWithSpaces_isSuccess() {
        assertTrue(responseIndicatesSuccess("""{"ok": true}""", "ok"))
    }

    @Test
    fun wdgwars_successField_isNotSuccess() {
        assertFalse(responseIndicatesSuccess("""{"success":true}""", "ok"))
    }

    @Test
    fun wigle_successJson_isSuccess() {
        assertTrue(responseIndicatesSuccess("""{"success":true}""", "success"))
    }

    @Test
    fun wigle_okField_isNotSuccess() {
        assertFalse(responseIndicatesSuccess("""{"ok":true}""", "success"))
    }

    @Test
    fun nonJson_fallsBackToSubstring() {
        assertTrue(responseIndicatesSuccess("prefix \"ok\":true suffix", "ok"))
    }

    @Test
    fun wdgwars_accepts200JsonOk() {
        assertTrue(wdgwarsUploadAccepted(200, """{"ok":true}"""))
    }

    @Test
    fun wdgwars_accepts200EmptyBody() {
        assertTrue(wdgwarsUploadAccepted(200, ""))
    }

    @Test
    fun wdgwars_rejects200JsonNotOk() {
        assertFalse(wdgwarsUploadAccepted(200, """{"ok":false}"""))
    }

    @Test
    fun wdgwars_rejects409EmptyBody() {
        assertFalse(wdgwarsUploadAccepted(409, ""))
    }

    @Test
    fun wdgwars_accepts409TypoProseDuplicate() {
        val body =
            "This file wasx already uploaded on 2026-05-29 18:57:31. " +
                "Same byte-for-byte ccontent. Skip or modify the file before retying."
        assertTrue(wdgwarsUploadAccepted(409, body))
    }

    @Test
    fun wdgwars_accepts409JsonDuplicateFlag() {
        assertTrue(wdgwarsUploadAccepted(409, """{"duplicate":true}"""))
    }

    @Test
    fun wdgwars_rejects409UnrelatedConflict() {
        assertFalse(wdgwarsUploadAccepted(409, "version conflict: resource locked"))
    }

    @Test
    fun wdgwars_accepts502WithJsonOk() {
        assertTrue(wdgwarsUploadAccepted(502, """{"ok":true}"""))
    }

    @Test
    fun wdgwars_rejects502Empty() {
        assertFalse(wdgwarsUploadAccepted(502, ""))
    }

    @Test
    fun wdgwars_rejects502Html() {
        assertFalse(wdgwarsUploadAccepted(502, "<html><body>Bad Gateway</body></html>"))
    }

    @Test
    fun wdgwars_accepts201OkAndDuplicate() {
        assertTrue(wdgwarsUploadAccepted(201, """{"ok":true,"duplicate":true}"""))
    }

    @Test
    fun validateWdgHttp200WithOk_succeeds() {
        val result = validateWdgUploadResponse(200, """{"ok":true}""")
        assertTrue(result.isSuccess)
    }

    @Test
    fun validateWdgHttp200WithoutOk_includesBody() {
        val result = validateWdgUploadResponse(200, """{"ok":false,"error":"bad"}""")
        assertTrue(result.isFailure)
        val msg = result.exceptionOrNull()?.message.orEmpty()
        assertTrue(msg.contains("wdgwars HTTP 200"))
        assertTrue(msg.contains("bad"))
    }

    @Test
    fun validateWdg409Duplicate_succeeds() {
        val result =
            validateWdgUploadResponse(
                409,
                "This file was already uploaded. Same byte-for-byte content.",
            )
        assertTrue(result.isSuccess)
    }

    @Test
    fun validateHttpError_includesStatusAndBody() {
        val result = validateWdgUploadResponse(401, """{"error":"bad key"}""")
        assertTrue(result.isFailure)
        assertEquals("wdgwars HTTP 401: {\"error\":\"bad key\"}", result.exceptionOrNull()?.message)
    }

    @Test
    fun sanitize_collapsesWhitespace() {
        assertEquals("a b c", sanitizeUploadLogText("a\nb\tc"))
    }
}
