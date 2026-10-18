package net.wardriver.mobile.data

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.FleetRosterEntity
import net.wardriver.mobile.db.ObservationEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ObservationDataClearInstrumentedTest {
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
    fun clearAllObservationData_wipesCaptureTables_preservesFleetRoster() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val sessionId =
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
            db.observationDao().insert(
                ObservationEntity(
                    sessionId = sessionId,
                    snifferId = 1,
                    observedUnixMs = System.currentTimeMillis(),
                    lat = 30.0,
                    lon = -97.0,
                    altitudeM = null,
                    accuracyM = 10f,
                    signalKind = "WIFI",
                    bssid = "AA:BB:CC:DD:EE:FF",
                    ssid = "test",
                    channel = 6,
                    frequencyMhz = 2437,
                    rssi = -50,
                    authMode = "[WPA2]",
                ),
            )
            db.fleetRosterDao().upsert(
                FleetRosterEntity(
                    snifferId = 1,
                    mac = "AA:BB:CC:DD:EE:01",
                    fleetSlot = 0,
                    role = "wifi",
                    band = "2.4",
                    pinnedChannel = null,
                    adopted = true,
                    adoptPending = false,
                    lastSeenMs = null,
                    cpuLoadPct = null,
                    heapAvailPct = null,
                    rebootCount = null,
                    state = null,
                ),
            )

            val repo = ObservationDataRepository(db)
            val result = repo.clearAllObservationData(context)

            assertTrue(result is ObservationDataRepository.ResetResult.Ok)
            assertEquals(0, db.observationDao().countAll())
            assertEquals(0, db.sessionDao().countAll())
            assertEquals(1, db.fleetRosterDao().adopted().size)
        }
}
