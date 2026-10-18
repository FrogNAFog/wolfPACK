package net.wardriver.mobile.ingest

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.NetworkSummaryEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionMapMarkerEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SessionMapMarkerFirstSeenInstrumentedTest {
    private lateinit var db: WardriverDatabase

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        db =
            Room.inMemoryDatabaseBuilder(context, WardriverDatabase::class.java)
                .allowMainThreadQueries()
                .build()
    }

    @Test
    fun sessionMapMarker_insertIgnore_preservesFirstSeenLocation() =
        runBlocking {
            val sessionId = insertSession()
            insertSessionMarker(
                sessionId = sessionId,
                bssid = "AA:BB:CC:DD:EE:FF",
                lat = 30.0,
                lon = -97.0,
                observedUnixMs = 1_000L,
            )
            insertSessionMarker(
                sessionId = sessionId,
                bssid = "AA:BB:CC:DD:EE:FF",
                lat = 31.0,
                lon = -98.0,
                observedUnixMs = 2_000L,
            )
            val snapshot =
                db.sessionMapMarkerDao()
                    .inViewportFlow(
                        sessionId = sessionId,
                        south = 29.0,
                        north = 32.0,
                        west = -99.0,
                        east = -96.0,
                    )
                    .first()
            assertEquals(1, snapshot.size)
            assertEquals(30.0, snapshot[0].lat, 0.0001)
            assertEquals(-97.0, snapshot[0].lon, 0.0001)
        }

    @Test
    fun networkSummary_updateLastOnly_preservesFirstSeenColumns() =
        runBlocking {
            db.networkSummaryDao().insertIgnore(
                NetworkSummaryEntity(
                    bssid = "AA:BB:CC:DD:EE:FF",
                    ssid = "first",
                    firstLat = 30.0,
                    firstLon = -97.0,
                    firstSeenUnixMs = 1_000L,
                    lastRssi = -50,
                    lastLat = 30.0,
                    lastLon = -97.0,
                    lastSeenUnixMs = 1_000L,
                    observationCount = 1,
                    lastSignalKind = "WIFI",
                ),
            )
            db.networkSummaryDao().updateLastOnly(
                bssid = "AA:BB:CC:DD:EE:FF",
                ssid = "revisit",
                lastRssi = -40,
                lastLat = 31.0,
                lastLon = -98.0,
                lastSeenUnixMs = 2_000L,
                observationCount = 2,
                lastSignalKind = "WIFI",
            )
            val summary = db.networkSummaryDao().all().single()
            assertEquals(30.0, summary.firstLat, 0.0001)
            assertEquals(-97.0, summary.firstLon, 0.0001)
            assertEquals(1_000L, summary.firstSeenUnixMs)
            assertEquals(31.0, summary.lastLat, 0.0001)
            assertNotEquals(summary.firstLat, summary.lastLat)
        }

    private suspend fun insertSession(): Long =
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

    private suspend fun insertSessionMarker(
        sessionId: Long,
        bssid: String,
        lat: Double,
        lon: Double,
        observedUnixMs: Long,
    ) {
        db.sessionMapMarkerDao().insert(
            SessionMapMarkerEntity(
                sessionId = sessionId,
                bssid = bssid,
                lat = lat,
                lon = lon,
                ssid = "ssid",
                rssi = -50,
                signalKind = "WIFI",
                observedUnixMs = observedUnixMs,
            ),
        )
    }
}
