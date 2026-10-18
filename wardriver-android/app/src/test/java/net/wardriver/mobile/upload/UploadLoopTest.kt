package net.wardriver.mobile.upload

import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.ObservationEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionWithCount
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UploadLoopTest {
    @Test
    fun wdgFailureStillRunsWiglePhase() =
        runBlocking {
            val observations = (1L..5L).map { sampleObs(it, sessionId = 1L) }
            var wiglePosts = 0
            val marked = mutableSetOf<Long>()

            val wdgConfig =
                UploadDestinationConfig(
                    dest = UploadDest.WDGWARS,
                    destId = DEST_WDGWARS,
                    nextPendingSessionId = { 1L },
                    pendingCountForSession = { _ -> 5 },
                    fetchSessionChunk = { _, afterId, limit ->
                        observations.filter { it.id > afterId }.take(limit)
                    },
                    postCsv = { _, _ -> Result.failure(IllegalStateException("wdg down")) },
                )
            val wigleConfig =
                UploadDestinationConfig(
                    dest = UploadDest.WIGLE,
                    destId = DEST_WIGLE,
                    nextPendingSessionId = {
                        if (marked.size < 5) 1L else null
                    },
                    pendingCountForSession = { _ -> 5 - marked.size },
                    fetchSessionChunk = { _, afterId, limit ->
                        observations.filter { it.id > afterId && it.id !in marked }.take(limit)
                    },
                    postCsv = { _, _ ->
                        wiglePosts += 1
                        Result.success(Unit)
                    },
                )

            val result =
                runSerializedUploadLoop(
                    destinations = listOf(wdgConfig, wigleConfig),
                    sessionDao = FakeSessionDao(),
                    uploadPrefs = InMemoryChunkStore(),
                    markBatch = { dest, ids ->
                        if (dest == UploadDest.WIGLE) {
                            marked.addAll(ids)
                        }
                    },
                    onProgress = { _, _, _, _, _ -> },
                    headers = "WigleWifi-1.6\nMAC,SSID\n",
                    lineFormatter = { _ -> "line" },
                )

            assertNotNull(result.resultFor(DEST_WDGWARS)?.error)
            assertNull(result.resultFor(DEST_WIGLE)?.error)
            assertTrue(wiglePosts > 0)
            assertEquals(5, result.resultFor(DEST_WIGLE)?.rowsUploaded)
            assertEquals(5, marked.size)
        }

    private fun sampleObs(id: Long, sessionId: Long): ObservationEntity =
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

    private class InMemoryChunkStore : UploadChunkIndexStore {
        private val chunks = mutableMapOf<String, Int>()

        override fun getChunkIndex(dest: UploadDest, sessionId: Long): Int =
            chunks["${dest.name}_$sessionId"] ?: 1

        override fun setChunkIndex(dest: UploadDest, sessionId: Long, index: Int) {
            chunks["${dest.name}_$sessionId"] = index
        }

        override fun clearChunkIndex(dest: UploadDest, sessionId: Long) {
            chunks.remove("${dest.name}_$sessionId")
        }
    }

    private class FakeSessionDao : net.wardriver.mobile.db.SessionDao {
        private val session =
            SessionEntity(
                id = 1L,
                startedUnixMs = 1_700_000_000_000L,
                endedUnixMs = 1_700_000_100_000L,
                gatewayMac = "",
                geofenceLatMin = null,
                geofenceLatMax = null,
                geofenceLonMin = null,
                geofenceLonMax = null,
            )

        override suspend fun insert(session: SessionEntity): Long = throw UnsupportedOperationException()

        override suspend fun update(session: SessionEntity) = Unit

        override suspend fun get(id: Long): SessionEntity? = if (id == 1L) session else null

        override fun getFlow(id: Long) = throw UnsupportedOperationException()

        override suspend fun getLatest(): SessionEntity? = session

        override suspend fun getOpenSessions(): List<SessionEntity> = emptyList()

        override suspend fun getLatestOpenOfKind(kind: String): SessionEntity? = null

        override suspend fun closeOrphansExcept(keepId: Long, endedMs: Long) = Unit

        override suspend fun updateSessionKind(id: Long, kind: String) = Unit

        override suspend fun updateGatewayMac(id: Long, mac: String) = Unit

        override suspend fun countAll(): Int = 1

        override suspend fun closeAllOpenSessions(endedMs: Long) = Unit

        override suspend fun countSessionsWithSameStartSecond(
            startedUnixMs: Long,
            sessionId: Long,
        ): Int = 0

        override suspend fun allSessionsWithCounts(): List<SessionWithCount> = emptyList()

        override suspend fun deleteAll() = Unit
    }
}
