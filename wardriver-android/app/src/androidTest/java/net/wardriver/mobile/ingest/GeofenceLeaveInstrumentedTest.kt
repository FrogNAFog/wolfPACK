package net.wardriver.mobile.ingest

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Simulates a GPS track crossing a wardrive geofence square edge.
 * Uses [GeofenceMonitor] directly (Kotlin fallback when arm64 JNI is absent on emulator).
 */
@RunWith(AndroidJUnit4::class)
class GeofenceLeaveInstrumentedTest {
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
    fun geofenceMonitor_firesLeft_onExitFromInside() {
        val latMin = 30.260
        val latMax = 30.270
        val lonMin = -97.750
        val lonMax = -97.740
        val monitor = GeofenceMonitor(latMin, latMax, lonMin, lonMax, enabled = true)

        val insideLat = 30.265
        val insideLon = -97.745
        assertEquals(GeofenceMonitor.GeofenceEvent.Entered, monitor.update(insideLat, insideLon))
        assertEquals(GeofenceMonitor.GeofenceEvent.None, monitor.update(insideLat, insideLon))

        val outsideLon = -97.735
        assertEquals(GeofenceMonitor.GeofenceEvent.Left, monitor.update(insideLat, outsideLon))
        assertEquals(GeofenceMonitor.GeofenceEvent.None, monitor.update(insideLat, outsideLon))
    }

    @Test
    fun geofenceMonitor_firesEntered_onReentryAfterLeave() {
        val monitor =
            GeofenceMonitor(
                latMin = 30.260,
                latMax = 30.270,
                lonMin = -97.750,
                lonMax = -97.740,
                enabled = true,
            )
        monitor.update(30.265, -97.745)
        monitor.update(30.265, -97.735)
        assertEquals(GeofenceMonitor.GeofenceEvent.Entered, monitor.update(30.265, -97.745))
    }

    @Test
    fun geofenceSession_storesBoundsOnSessionRow() = runBlocking {
        val sessionId =
            db.sessionDao().insert(
                SessionEntity(
                    startedUnixMs = System.currentTimeMillis(),
                    endedUnixMs = null,
                    gatewayMac = "",
                    geofenceLatMin = 30.260,
                    geofenceLatMax = 30.270,
                    geofenceLonMin = -97.750,
                    geofenceLonMax = -97.740,
                ),
            )
        val session = db.sessionDao().get(sessionId)
        assertEquals(30.260, session?.geofenceLatMin)
        assertEquals(-97.740, session?.geofenceLonMax)
    }
}
