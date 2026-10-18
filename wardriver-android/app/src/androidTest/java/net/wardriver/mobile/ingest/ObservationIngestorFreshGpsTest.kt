package net.wardriver.mobile.ingest

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ObservationIngestorFreshGpsTest {
    private lateinit var db: WardriverDatabase

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        db =
            Room.inMemoryDatabaseBuilder(context, WardriverDatabase::class.java)
                .allowMainThreadQueries()
                .build()
    }

    @After
    fun tearDown() {
        db.close()
    }

    @Test
    fun requireInitialFreshGpsFalse_opensImmediately() = runBlocking {
        val ing = ObservationIngestor(db = db, sessionId = 1L, requireInitialFreshGps = false)
        assertTrue(ing.isCsvGateOpen())
        ing.shutdown()
    }

    @Test
    fun requireInitialFreshGpsTrue_latchesOnFreshUsableFix() = runBlocking {
        val ing = ObservationIngestor(db = db, sessionId = 1L, requireInitialFreshGps = true)
        val t0 = System.currentTimeMillis()
        assertFalse(ing.isCsvGateOpen())
        ing.onGpsFix(t0, 37.7749, -122.4194, 8f, isFresh = true)
        assertTrue(ing.isCsvGateOpen())
        ing.shutdown()
    }

    @Test
    fun staleFixDoesNotLatch() = runBlocking {
        val ing = ObservationIngestor(db = db, sessionId = 1L, requireInitialFreshGps = true)
        val t0 = System.currentTimeMillis()
        ing.onGpsFix(t0, 37.7749, -122.4194, 8f, isFresh = false)
        assertFalse(ing.isCsvGateOpen())
        ing.shutdown()
    }

    @Test
    fun badFixAfterLatchDoesNotCloseGate() = runBlocking {
        val ing = ObservationIngestor(db = db, sessionId = 1L, requireInitialFreshGps = true)
        val t0 = System.currentTimeMillis()
        ing.onGpsFix(t0, 37.7749, -122.4194, 8f, isFresh = true)
        assertTrue(ing.isCsvGateOpen())
        ing.onGpsFix(t0 + 1_000, 37.7749, -122.4194, 99f, isFresh = false)
        assertTrue(ing.isCsvGateOpen())
        ing.shutdown()
    }
}
