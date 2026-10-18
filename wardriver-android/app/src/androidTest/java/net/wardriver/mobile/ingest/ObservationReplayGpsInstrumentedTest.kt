package net.wardriver.mobile.ingest

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.db.RoutePointEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ObservationReplayGpsInstrumentedTest {
    private lateinit var db: WardriverDatabase
    private var sessionId: Long = 0
    private var ingestor: ObservationIngestor? = null

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        db =
            Room.inMemoryDatabaseBuilder(context, WardriverDatabase::class.java)
                .allowMainThreadQueries()
                .build()
        sessionId =
            runBlocking {
                db.sessionDao().insert(
                    SessionEntity(
                        startedUnixMs = System.currentTimeMillis(),
                        endedUnixMs = null,
                        gatewayMac = "",
                        geofenceLatMin = null,
                        geofenceLatMax = null,
                        geofenceLonMin = null,
                        geofenceLonMax = null,
                    ),
                )
            }
    }

    @After
    fun tearDown() {
        ingestor?.shutdown()
        ingestor = null
        db.close()
    }

    private fun newIngestor(): ObservationIngestor {
        val ing =
            ObservationIngestor(
                db = db,
                sessionId = sessionId,
            )
        ingestor = ing
        return ing
    }

    @Test
    fun routeDao_betweenUnixMs_spansSessions() = runBlocking {
        val now = System.currentTimeMillis()
        val otherSession =
            db.sessionDao().insert(
                SessionEntity(
                    startedUnixMs = now - 120_000L,
                    endedUnixMs = now - 60_000L,
                    gatewayMac = "",
                    geofenceLatMin = null,
                    geofenceLatMax = null,
                    geofenceLonMin = null,
                    geofenceLonMax = null,
                ),
            )
        val fixTime = now - 30_000L
        db.routeDao().insert(
            RoutePointEntity(
                sessionId = otherSession,
                unixMs = fixTime,
                lat = 37.7749,
                lon = -122.4194,
                accuracyM = 8f,
            ),
        )
        val points = db.routeDao().betweenUnixMs(now - GpsTimeline.GPS_HIST_RETENTION_MS, now)
        assertEquals(1, points.size)
        assertEquals(fixTime, points[0].unixMs)
    }

    @Test
    fun replayHydrationFailure_throws() {
        assumeTrue(nativeCoreLoaded())
        val ing = newIngestor()
        db.close()
        val thrown =
            assertThrows(IllegalStateException::class.java) {
                runBlocking {
                    ing.onReplayPackLinkFrame(ObservationBulkTestFixtures.WIRE)
                }
            }
        assertTrue(thrown.message?.contains("hydration") == true)
    }

    @Test
    fun replayOlderThanRetention_recordsStaleReplay() = runBlocking {
        assumeTrue(nativeCoreLoaded())
        val ing = newIngestor()
        ing.openSession()
        ObservationIngestStats.reset()
        ing.onReplayPackLinkFrame(ObservationBulkTestFixtures.WIRE)
        val snap = ObservationIngestStats.snapshot.value
        assertTrue((snap.dropReasons[ObservationIngestor.STALE_REPLAY_REASON] ?: 0L) >= 1L)
    }

    @Test
    fun replayHydration_findsRouteWithinMatchWindow() = runBlocking {
        assumeTrue(nativeCoreLoaded())
        val anchor = ObservationBulkTestFixtures.SCAN_END_UNIX_MS
        val now = System.currentTimeMillis()
        assumeTrue(!GpsTimeline.isStaleReplay(anchor, now))
        db.routeDao().insert(
            RoutePointEntity(
                sessionId = sessionId,
                unixMs = anchor,
                lat = 37.7749,
                lon = -122.4194,
                accuracyM = 8f,
            ),
        )
        val ing = newIngestor()
        ing.openSession()
        ObservationIngestStats.reset()
        ing.onReplayPackLinkFrame(ObservationBulkTestFixtures.WIRE)
        val stored = db.observationDao().countForSession(sessionId)
        assertEquals(1, stored)
    }

    private fun nativeCoreLoaded(): Boolean =
        try {
            NativeCore.packlinkMagic()
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
}

/** Golden PackLink ObservationBulk from `wardriver-mobile-core` pipeline_wifi_parity. */
private object ObservationBulkTestFixtures {
    const val SCAN_END_UNIX_MS = 1_777_190_400_000L

    val WIRE: ByteArray by lazy {
        hexToBytes(
            "504c4e4b0105020051000100000014000000030000000070cdc89d010000010000000603574452" +
                "565244573201030000000a00000014000000ddccbbaa50c300000070cdc89d010000000101" +
                "aabbccddeeff06c90305536e6f6f7050c30000c824f4e1",
        )
    }

    private fun hexToBytes(hex: String): ByteArray {
        val clean = hex.trim()
        return ByteArray(clean.length / 2) { i ->
            clean.substring(i * 2, i * 2 + 2).toInt(16).toByte()
        }
    }
}
