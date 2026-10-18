package net.wardriver.mobile.ingest

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.db.RoutePointEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionKind
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CatchUpReplayGpsInstrumentedTest {
    private lateinit var db: WardriverDatabase
    private var sessionId: Long = 0

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
                        gatewayMac = "aa:bb:cc:dd:ee:ff",
                        sessionKind = SessionKind.CATCH_UP,
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
        db.close()
    }

    @Test
    fun replayWithRouteButNoLiveGps_storesObservation() = runBlocking {
        assumeTrue(nativeCoreLoaded())
        val anchor = ObservationBulkTestFixtures.SCAN_END_UNIX_MS
        db.routeDao().insert(
            RoutePointEntity(
                sessionId = sessionId,
                unixMs = anchor,
                lat = 37.7749,
                lon = -122.4194,
                accuracyM = 8f,
            ),
        )
        val ing = ObservationIngestor(db = db, sessionId = sessionId, requireInitialFreshGps = false)
        ing.openSession()
        ing.onReplayPackLinkFrame(ObservationBulkTestFixtures.WIRE, failOnInvalid = false)
        assertEquals(1, db.observationDao().countForSession(sessionId))
        ing.shutdown()
    }

    @Test
    fun replayWithRouteAndLiveGps_storesObservation() = runBlocking {
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
        val ing = ObservationIngestor(db = db, sessionId = sessionId, requireInitialFreshGps = false)
        ing.openSession()
        ing.onGpsFix(now - 1_000, 37.7749, -122.4194, 8f, isFresh = true)
        assertEquals(true, ing.isCsvGateOpen())
        ing.onReplayPackLinkFrame(ObservationBulkTestFixtures.WIRE, failOnInvalid = false)
        assertEquals(1, db.observationDao().countForSession(sessionId))
        ing.shutdown()
    }

    @Test
    fun newWardriveWithoutFreshGps_storesNothing() = runBlocking {
        assumeTrue(nativeCoreLoaded())
        val anchor = ObservationBulkTestFixtures.SCAN_END_UNIX_MS
        val wardriveSessionId =
            db.sessionDao().insert(
                SessionEntity(
                    startedUnixMs = System.currentTimeMillis(),
                    endedUnixMs = null,
                    gatewayMac = "aa:bb:cc:dd:ee:ff",
                    sessionKind = SessionKind.WARDRIVE,
                    geofenceLatMin = null,
                    geofenceLatMax = null,
                    geofenceLonMin = null,
                    geofenceLonMax = null,
                ),
            )
        db.routeDao().insert(
            RoutePointEntity(
                sessionId = wardriveSessionId,
                unixMs = anchor,
                lat = 37.7749,
                lon = -122.4194,
                accuracyM = 8f,
            ),
        )
        val ing =
            ObservationIngestor(
                db = db,
                sessionId = wardriveSessionId,
                requireInitialFreshGps = true,
            )
        ing.openSession()
        assertFalse(ing.isCsvGateOpen())
        ing.onGpsFix(System.currentTimeMillis(), 37.7749, -122.4194, 8f, isFresh = false)
        assertFalse(ing.isCsvGateOpen())
        ing.onReplayPackLinkFrame(ObservationBulkTestFixtures.WIRE, failOnInvalid = false)
        assertEquals(0, db.observationDao().countForSession(wardriveSessionId))
        ing.shutdown()
    }

    private fun nativeCoreLoaded(): Boolean =
        try {
            NativeCore.packlinkMagic()
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
}

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
