package net.wardriver.mobile.ingest

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.wardriver.mobile.db.RoutePointEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.WardriverDatabase
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class GpsTimelineInstrumentedTest {
    private lateinit var db: WardriverDatabase
    private val openTimelines = mutableListOf<GpsTimeline>()

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
        openTimelines.forEach { it.close() }
        openTimelines.clear()
    }

    private fun newTimeline(): GpsTimeline = GpsTimeline().also { openTimelines.add(it) }

    @Test
    fun append_usesFixTimestamp() {
        val now = System.currentTimeMillis()
        val fixTime = now - 1_500L
        val timeline = newTimeline()
        assertTrue(timeline.append(fixTime, 37.77, -122.42, 8f, 12.0, now))
        val json = timeline.toJsonArray().getJSONObject(0)
        assertEquals(fixTime, json.getLong("unix_ms"))
        assertEquals(8.0, json.getDouble("accuracy_m"), 0.001)
        assertFalse(json.has("hdop"))
    }

    @Test
    fun append_rejectsNonPositiveTimestamp() {
        val timeline = newTimeline()
        assertFalse(timeline.append(0L, 37.77, -122.42, 8f, null))
        assertEquals(0, timeline.size())
    }

    @Test
    fun append_rejectsFutureSkew() {
        val now = System.currentTimeMillis()
        val timeline = newTimeline()
        assertFalse(timeline.append(now + 120_000L, 37.77, -122.42, 8f, null, now))
        assertEquals(0, timeline.size())
    }

    @Test
    fun append_deduplicatesEqualTimestamp() {
        val now = System.currentTimeMillis()
        val fixTime = now - 500L
        val timeline = newTimeline()
        timeline.append(fixTime, 37.77, -122.42, 8f, null, now)
        timeline.append(fixTime, 37.78, -122.43, 6f, null, now)
        assertEquals(1, timeline.size())
        assertEquals(37.78, timeline.toJsonArray().getJSONObject(0).getDouble("lat"), 0.0001)
    }

    @Test
    fun append_sortsOutOfOrderBatch() {
        val now = System.currentTimeMillis()
        val timeline = newTimeline()
        timeline.append(now - 1_000L, 37.77, -122.42, 8f, null, now)
        timeline.append(now - 2_000L, 37.76, -122.41, 8f, null, now)
        timeline.append(now - 500L, 37.78, -122.43, 8f, null, now)
        val arr = timeline.toJsonArray()
        assertEquals(now - 2_000L, arr.getJSONObject(0).getLong("unix_ms"))
        assertEquals(now - 1_000L, arr.getJSONObject(1).getLong("unix_ms"))
        assertEquals(now - 500L, arr.getJSONObject(2).getLong("unix_ms"))
    }

    @Test
    fun prune_retainsFiveMinutes() {
        val now = System.currentTimeMillis()
        val timeline = newTimeline()
        timeline.append(now - GpsTimeline.GPS_HIST_RETENTION_MS - 1_000L, 37.77, -122.42, 8f, null, now)
        timeline.append(now - 30_000L, 37.78, -122.43, 8f, null, now)
        timeline.prune(now)
        assertEquals(1, timeline.size())
        assertEquals(now - 30_000L, timeline.toJsonArray().getJSONObject(0).getLong("unix_ms"))
    }

    @Test
    fun stationarySamples_accumulateAt500msCadence() {
        val now = System.currentTimeMillis()
        val timeline = newTimeline()
        repeat(12) { i ->
            timeline.append(now - (11 - i) * 500L, 37.77, -122.42, 8f, null, now)
        }
        assertEquals(12, timeline.size())
    }

    @Test
    fun isStaleReplay_detectsOlderThanRetention() {
        val now = System.currentTimeMillis()
        val anchor = now - GpsTimeline.GPS_HIST_RETENTION_MS - 1L
        assertTrue(GpsTimeline.isStaleReplay(anchor, now))
        assertFalse(GpsTimeline.isStaleReplay(now - 30_000L, now))
    }

    @Test
    fun mergeRoutePoints_hydratesTimeline() = runBlocking {
        val now = System.currentTimeMillis()
        val fixTime = now - 45_000L
        val sessionId =
            db.sessionDao().insert(
                SessionEntity(
                    startedUnixMs = now - 60_000L,
                    endedUnixMs = null,
                    gatewayMac = "",
                    geofenceLatMin = null,
                    geofenceLatMax = null,
                    geofenceLonMin = null,
                    geofenceLonMax = null,
                ),
            )
        db.routeDao().insert(
            RoutePointEntity(
                sessionId = sessionId,
                unixMs = fixTime,
                lat = 37.7749,
                lon = -122.4194,
                accuracyM = 10f,
            ),
        )
        val points = db.routeDao().betweenUnixMs(now - GpsTimeline.GPS_HIST_RETENTION_MS, now)
        val timeline = newTimeline()
        timeline.mergeRoutePoints(points, now)
        assertEquals(1, timeline.size())
        assertEquals(fixTime, timeline.toJsonArray().getJSONObject(0).getLong("unix_ms"))
    }
}
