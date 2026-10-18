package net.wardriver.mobile.data

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.DetectionEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class GeofenceDetectionPurgeInstrumentedTest {
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
    fun deleteGeofenceTransitions_removesOnlyGeofenceRows() =
        runBlocking {
            db.detectionDao().insert(
                DetectionEntity(
                    sessionId = 1L,
                    kind = "GEOFENCE_LEAVE",
                    unixMs = 1_000L,
                    detail = "lat=1.0,lon=2.0",
                ),
            )
            db.detectionDao().insert(
                DetectionEntity(
                    sessionId = 1L,
                    kind = "FLOCK_WIFI",
                    unixMs = 2_000L,
                    detail = "mac=aa:bb:cc:dd:ee:01",
                ),
            )

            assertEquals(1, db.detectionDao().deleteGeofenceTransitions())
            assertEquals(1, db.detectionDao().countAll())
        }
}
