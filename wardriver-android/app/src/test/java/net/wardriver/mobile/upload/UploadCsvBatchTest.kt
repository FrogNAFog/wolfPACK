package net.wardriver.mobile.upload

import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.ObservationEntity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class UploadCsvBatchTest {
    @Test
    fun emptyInput_returnsEmptyBatch() {
        val batch = buildUploadBatch(emptyList(), HEADER, 10_000L) { "line" }
        assertEquals(0, batch.rowCount)
        assertTrue(batch.uploadedIds.isEmpty())
    }

    @Test
    fun stopsBeforeExceedingMaxBytes() {
        val observations =
            (1L..20L).map { id ->
                sampleObs(id)
            }
        val batch =
            buildUploadBatch(observations, HEADER, 500L) { _ ->
                "x".repeat(80)
            }
        assertTrue(batch.rowCount in 1 until 20)
        val bytes = batch.csvText.toByteArray(Charsets.UTF_8).size
        assertTrue(bytes <= 500)
    }

    @Test
    fun oversizedSingleRow_stillProducesBatch() {
        val batch =
            buildUploadBatch(
                listOf(sampleObs(1L)),
                HEADER,
                maxBytes = 50L,
                lineFormatter = { "y".repeat(200) },
            )
        assertEquals(1, batch.rowCount)
        assertEquals(1L, batch.uploadedIds.single())
    }

    @Test
    fun skipsNullFormattedLines() {
        val batch =
            buildUploadBatch(
                listOf(sampleObs(1L), sampleObs(2L)),
                HEADER,
                maxBytes = 10_000L,
                lineFormatter = { obs ->
                    if (obs.id == 1L) null else "ok"
                },
            )
        assertEquals(1, batch.rowCount)
        assertEquals(2L, batch.uploadedIds.single())
    }

    @Test
    fun buildsLargeRowCountWhenUnderByteCap() {
        val observations = (1L..2000L).map { id -> sampleObs(id) }
        val batch =
            buildUploadBatch(observations, HEADER, maxBytes = 100L * 1024L * 1024L) { _ ->
                "x"
            }
        assertEquals(2000, batch.rowCount)
        assertEquals(2000, batch.uploadedIds.size)
    }

    @Test
    fun assembleBatchForSession_doesNotMixSessions() =
        runBlocking {
            val sessionOne = (1L..10L).map { sampleObs(it, sessionId = 1L) }
            val sessionTwo = (11L..20L).map { sampleObs(it, sessionId = 2L) }
            val all = sessionOne + sessionTwo
            val batch =
                assembleBatchForSession(
                    sessionId = 1L,
                    headers = HEADER,
                    maxBytes = 100_000L,
                    fetchChunk = { sid, afterId, limit ->
                        all.filter { it.sessionId == sid && it.id > afterId }.take(limit)
                    },
                    lineFormatter = { _ -> "line" },
                )
            assertEquals(10, batch.rowCount)
            assertTrue(batch.uploadedIds.all { id -> id <= 10L })
        }

    @Test
    fun assembleBatchFromMarker_stopsAtByteCapWithoutLoadingAll() =
        runBlocking {
            val allObs = (1L..10_000L).map { sampleObs(it) }
            var fetchCalls = 0
            val batch =
                assembleBatchFromMarker(
                    marker = 0L,
                    headers = HEADER,
                    maxBytes = 2_000L,
                    fetchChunk = { marker, limit ->
                        fetchCalls += 1
                        allObs.filter { it.id > marker }.take(limit)
                    },
                    lineFormatter = { _ -> "x".repeat(80) },
                )
            assertTrue(batch.rowCount in 1 until 10_000)
            assertTrue(fetchCalls < 10)
        }

    @Test
    fun assembleBatchFromMarker_formatsEachRowOnce() =
        runBlocking {
            val observations = (1L..50L).map { sampleObs(it) }
            var formatCalls = 0
            val batch =
                assembleBatchFromMarker(
                    marker = 0L,
                    headers = HEADER,
                    maxBytes = 100_000L,
                    fetchChunk = { marker, limit ->
                        observations.filter { it.id > marker }.take(limit)
                    },
                    lineFormatter = {
                        formatCalls += 1
                        "line"
                    },
                )
            assertEquals(batch.rowCount, formatCalls)
            assertEquals(50, batch.rowCount)
        }

    private fun sampleObs(id: Long, sessionId: Long = 1L): ObservationEntity =
        ObservationEntity(
            id = id,
            sessionId = sessionId,
            snifferId = 0,
            observedUnixMs = 1_700_000_000_000L,
            lat = 37.0,
            lon = -122.0,
            altitudeM = null,
            accuracyM = 5f,
            signalKind = "WIFI",
            bssid = "aa:bb:cc:dd:ee:ff",
            ssid = "test",
            channel = 6,
            frequencyMhz = 2437,
            rssi = -50,
            authMode = "[WPA2]",
        )

    companion object {
        private const val HEADER = "WigleWifi-1.6\nMAC,SSID\n"
    }
}
