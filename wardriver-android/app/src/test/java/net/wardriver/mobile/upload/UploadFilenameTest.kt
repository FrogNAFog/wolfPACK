package net.wardriver.mobile.upload

import net.wardriver.mobile.db.SessionEntity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant

class UploadFilenameTest {
    private val startedMs = Instant.parse("2024-07-26T17:30:00Z").toEpochMilli()

    @Test
    fun formatsIsoUtcWithoutChunkSuffixForFirstChunk() {
        val session = sessionAt(startedMs, id = 1L)
        val name = uploadCsvFileName(session, chunkIndex = 1, useSessionIdInfix = false)
        assertEquals("2024-07-26T17-30-00Z.csv", name)
    }

    @Test
    fun addsChunkSuffixFromSecondChunk() {
        val session = sessionAt(startedMs, id = 2L)
        assertEquals(
            "2024-07-26T17-30-00Z_02.csv",
            uploadCsvFileName(session, chunkIndex = 2, useSessionIdInfix = false),
        )
        assertEquals(
            "2024-07-26T17-30-00Z_03.csv",
            uploadCsvFileName(session, chunkIndex = 3, useSessionIdInfix = false),
        )
    }

    @Test
    fun addsSessionIdInfixOnCollision() {
        val session = sessionAt(startedMs, id = 42L)
        val name = uploadCsvFileName(session, chunkIndex = 1, useSessionIdInfix = true)
        assertTrue(name.contains("_s42"))
        assertEquals("2024-07-26T17-30-00Z_s42.csv", name)
    }

    private fun sessionAt(startedUnixMs: Long, id: Long): SessionEntity =
        SessionEntity(
            id = id,
            startedUnixMs = startedUnixMs,
            endedUnixMs = null,
            gatewayMac = "",
            geofenceLatMin = null,
            geofenceLatMax = null,
            geofenceLonMin = null,
            geofenceLonMax = null,
        )
}
