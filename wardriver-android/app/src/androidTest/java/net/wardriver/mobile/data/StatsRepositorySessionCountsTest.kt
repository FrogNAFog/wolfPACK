package net.wardriver.mobile.data

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.ObservationEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.wardriverApp
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class StatsRepositorySessionCountsTest {
    private lateinit var db: WardriverDatabase
    private lateinit var statsRepository: StatsRepository
    private var activeSessionId: Long? = null

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val app = context.wardriverApp()
        db =
            Room.inMemoryDatabaseBuilder(context, WardriverDatabase::class.java)
                .allowMainThreadQueries()
                .build()
        statsRepository =
            StatsRepository(
                db = db,
                gatewayWifi = app.gatewayWifiRepository,
                appPreferences = app.appPreferences,
                gpsLocation = { null },
                activeSessionId = { activeSessionId },
            )
    }

    @Test
    fun refresh_sessionScopedCounts_matchActiveSessionOnly() =
        runBlocking {
            val session1 = insertSession()
            val session2 = insertSession()

            insertObservation(session1, "WIFI", "AA:BB:CC:DD:EE:01")
            insertObservation(session1, "BLE", "AA:BB:CC:DD:EE:02")
            insertObservation(session1, "LTE", "310:260:12345:67890")

            insertObservation(session2, "WIFI", "AA:BB:CC:DD:EE:01")
            insertObservation(session2, "WIFI", "AA:BB:CC:DD:EE:03")
            insertObservation(session2, "BLE", "AA:BB:CC:DD:EE:04")
            insertObservation(session2, "GSM", "310:260:99999:11111")

            activeSessionId = session2
            statsRepository.refresh()
            val active = statsRepository.stats.value
            assertEquals(4, active.observationRowCount)
            assertEquals(2, active.uniqueWifiMacs)
            assertEquals(1, active.uniqueBleDevices)
            assertEquals(1, active.uniqueCellTowers)

            activeSessionId = null
            statsRepository.refresh()
            val idle = statsRepository.stats.value
            assertEquals(0, idle.observationRowCount)
            assertEquals(0, idle.uniqueWifiMacs)
            assertEquals(0, idle.uniqueBleDevices)
            assertEquals(0, idle.uniqueCellTowers)
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

    private suspend fun insertObservation(
        sessionId: Long,
        signalKind: String,
        bssid: String,
    ) {
        db.observationDao().insert(
            ObservationEntity(
                sessionId = sessionId,
                snifferId = 1,
                observedUnixMs = System.currentTimeMillis(),
                lat = 30.0,
                lon = -97.0,
                altitudeM = null,
                accuracyM = 10f,
                signalKind = signalKind,
                bssid = bssid,
                ssid = "test",
                channel = 6,
                frequencyMhz = 2437,
                rssi = -50,
                authMode = "[WPA2]",
            ),
        )
    }
}
