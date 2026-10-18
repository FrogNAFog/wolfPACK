package net.wardriver.mobile.upload

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.Assert.assertNotNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class UploadSessionCoordinatorInstrumentedTest {
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
    fun closeAllOpenSessions_setsEndedUnixMs() =
        runBlocking {
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
            db.sessionDao().closeAllOpenSessions(System.currentTimeMillis())
            val closed = db.sessionDao().get(sessionId)
            assertNotNull(closed?.endedUnixMs)
        }

    @Test
    fun countSessionsWithSameStartSecond_detectsCollision() =
        runBlocking {
            val started = 1_722_018_600_000L
            val a =
                db.sessionDao().insert(
                    SessionEntity(
                        startedUnixMs = started,
                        endedUnixMs = started + 1,
                        gatewayMac = "",
                        geofenceLatMin = null,
                        geofenceLatMax = null,
                        geofenceLonMin = null,
                        geofenceLonMax = null,
                    ),
                )
            val b =
                db.sessionDao().insert(
                    SessionEntity(
                        startedUnixMs = started + 500,
                        endedUnixMs = started + 1,
                        gatewayMac = "",
                        geofenceLatMin = null,
                        geofenceLatMax = null,
                        geofenceLonMin = null,
                        geofenceLonMax = null,
                    ),
                )
            val collisions = db.sessionDao().countSessionsWithSameStartSecond(started, b)
            assertNotNull(collisions)
            org.junit.Assert.assertTrue(collisions > 0)
            val alone = db.sessionDao().countSessionsWithSameStartSecond(started, a)
            org.junit.Assert.assertEquals(1, alone)
        }
}
