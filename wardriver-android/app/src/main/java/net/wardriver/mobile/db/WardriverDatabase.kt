package net.wardriver.mobile.db

import android.content.Context
import androidx.room.ColumnInfo
import androidx.room.Dao
import androidx.room.Database
import androidx.room.Embedded
import androidx.room.Entity
import androidx.room.Index
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.PrimaryKey
import androidx.room.Query
import androidx.room.Delete
import androidx.room.Room
import androidx.room.RoomDatabase
import androidx.room.Update
import androidx.room.Upsert
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase
import kotlinx.coroutines.flow.Flow

/** Session with observation count for session picker UI. */
data class SessionWithCount(
    @Embedded val session: SessionEntity,
    @ColumnInfo(name = "obs_count") val observationCount: Int,
)

/** Append-only primary store: one row per allowed sighting. */
@Entity(
    tableName = "observation",
    indices = [
        Index("bssid"),
        Index("session_id"),
        Index("uploaded"),
        Index("uploaded_wigle"),
        Index("uploaded_wdgwars"),
        // Must match MIGRATION_7_8 composite indexes (session-scoped upload queries).
        Index(value = ["session_id", "uploaded_wigle"]),
        Index(value = ["session_id", "uploaded_wdgwars"]),
    ],
)
data class ObservationEntity(
    @PrimaryKey(autoGenerate = true) @ColumnInfo(name = "_id") val id: Long = 0,
    @ColumnInfo(name = "session_id") val sessionId: Long,
    @ColumnInfo(name = "sniffer_id") val snifferId: Int,
    @ColumnInfo(name = "observed_unix_ms") val observedUnixMs: Long,
  @ColumnInfo(name = "lat") val lat: Double,
    @ColumnInfo(name = "lon") val lon: Double,
    @ColumnInfo(name = "altitude_m") val altitudeM: Int?,
    @ColumnInfo(name = "accuracy_m") val accuracyM: Float?,
    @ColumnInfo(name = "signal_kind") val signalKind: String,
    @ColumnInfo(name = "bssid") val bssid: String,
    @ColumnInfo(name = "ssid") val ssid: String,
    @ColumnInfo(name = "channel") val channel: Int,
    @ColumnInfo(name = "frequency_mhz") val frequencyMhz: Int,
    @ColumnInfo(name = "rssi") val rssi: Int,
    @ColumnInfo(name = "auth_mode") val authMode: String,
    @ColumnInfo(name = "rcois", defaultValue = "''") val rcois: String = "",
    @ColumnInfo(name = "ble_capabilities", defaultValue = "''") val bleCapabilities: String = "",
    @ColumnInfo(name = "uploaded") val uploaded: Boolean = false,
    @ColumnInfo(name = "uploaded_wigle", defaultValue = "0") val uploadedWigle: Boolean = false,
    @ColumnInfo(name = "uploaded_wdgwars", defaultValue = "0") val uploadedWdgwars: Boolean = false,
)

@Entity(
    tableName = "network_summary",
    indices = [
        Index("bssid", unique = true),
        Index(value = ["first_lat", "first_lon"]),
    ],
)
data class NetworkSummaryEntity(
    @PrimaryKey @ColumnInfo(name = "bssid") val bssid: String,
    @ColumnInfo(name = "ssid") val ssid: String,
    @ColumnInfo(name = "first_lat") val firstLat: Double,
    @ColumnInfo(name = "first_lon") val firstLon: Double,
    @ColumnInfo(name = "first_seen_unix_ms") val firstSeenUnixMs: Long,
    @ColumnInfo(name = "last_rssi") val lastRssi: Int,
    @ColumnInfo(name = "last_lat") val lastLat: Double,
    @ColumnInfo(name = "last_lon") val lastLon: Double,
    @ColumnInfo(name = "last_seen_unix_ms") val lastSeenUnixMs: Long,
    @ColumnInfo(name = "observation_count") val observationCount: Int,
    @ColumnInfo(name = "last_signal_kind", defaultValue = "WIFI") val lastSignalKind: String = "WIFI",
)

/** First-seen map pin per BSSID within a wardrive session. */
@Entity(
    tableName = "session_map_marker",
    primaryKeys = ["session_id", "bssid"],
    indices = [Index(value = ["session_id", "lat", "lon"])],
)
data class SessionMapMarkerEntity(
    @ColumnInfo(name = "session_id") val sessionId: Long,
    @ColumnInfo(name = "bssid") val bssid: String,
    @ColumnInfo(name = "lat") val lat: Double,
    @ColumnInfo(name = "lon") val lon: Double,
    @ColumnInfo(name = "ssid") val ssid: String,
    @ColumnInfo(name = "rssi") val rssi: Int,
    @ColumnInfo(name = "signal_kind", defaultValue = "WIFI") val signalKind: String = "WIFI",
    @ColumnInfo(name = "observed_unix_ms") val observedUnixMs: Long,
)

@Entity(tableName = "route")
data class RoutePointEntity(
    @PrimaryKey(autoGenerate = true) @ColumnInfo(name = "_id") val id: Long = 0,
    @ColumnInfo(name = "session_id") val sessionId: Long,
    @ColumnInfo(name = "unix_ms") val unixMs: Long,
    @ColumnInfo(name = "lat") val lat: Double,
    @ColumnInfo(name = "lon") val lon: Double,
    @ColumnInfo(name = "accuracy_m") val accuracyM: Float?,
)

/** `wardrive` = active foreground session; `catch_up` = gateway-connected import session. */
object SessionKind {
    const val WARDRIVE = "wardrive"
    const val CATCH_UP = "catch_up"
}

@Entity(tableName = "session")
data class SessionEntity(
    @PrimaryKey(autoGenerate = true) @ColumnInfo(name = "_id") val id: Long = 0,
    @ColumnInfo(name = "started_unix_ms") val startedUnixMs: Long,
    @ColumnInfo(name = "ended_unix_ms") val endedUnixMs: Long?,
    @ColumnInfo(name = "gateway_mac") val gatewayMac: String,
    @ColumnInfo(name = "session_kind", defaultValue = SessionKind.WARDRIVE) val sessionKind: String = SessionKind.WARDRIVE,
    @ColumnInfo(name = "geofence_lat_min") val geofenceLatMin: Double?,
    @ColumnInfo(name = "geofence_lat_max") val geofenceLatMax: Double?,
    @ColumnInfo(name = "geofence_lon_min") val geofenceLonMin: Double?,
    @ColumnInfo(name = "geofence_lon_max") val geofenceLonMax: Double?,
)

/** Phone-owned fleet roster row (adopted sniffers + live SoftAP status fields). */
@Entity(tableName = "fleet_roster")
data class FleetRosterEntity(
    @PrimaryKey @ColumnInfo(name = "sniffer_id") val snifferId: Int,
    @ColumnInfo(name = "mac") val mac: String,
    @ColumnInfo(name = "fleet_slot") val fleetSlot: Int,
    /** `wifi` or `ble` (wolfPACK [`SnifferRole`] wire values 0/1). */
    @ColumnInfo(name = "role") val role: String,
    /** Display band label (MVP: `2.4` until 5 GHz policy lands). */
    @ColumnInfo(name = "band") val band: String,
    @ColumnInfo(name = "pinned_channel") val pinnedChannel: Int?,
    @ColumnInfo(name = "adopted") val adopted: Boolean,
    /** PolicySync pushed; waiting for sniffer-bound `adoption_state=ADOPTED`. */
    @ColumnInfo(name = "adopt_pending", defaultValue = "0") val adoptPending: Boolean = false,
    /** Gateway mono `last_seen_ms` from PackLink `SnifferStatus`; null until first notify. */
    @ColumnInfo(name = "last_seen_ms") val lastSeenMs: Long?,
    /** 0-100 from `SnifferStatus`; null until first notify (UI shows em dash). */
    @ColumnInfo(name = "cpu_load_pct") val cpuLoadPct: Int?,
    @ColumnInfo(name = "heap_avail_pct") val heapAvailPct: Int?,
    @ColumnInfo(name = "reboot_count") val rebootCount: Int?,
    /** wolfPACK `SnifferState` u8; null until first notify. */
    @ColumnInfo(name = "state") val state: Int?,
)

@Entity(
    tableName = "detection",
    indices = [Index(value = ["lat", "lon"])],
)
data class DetectionEntity(
    @PrimaryKey(autoGenerate = true) @ColumnInfo(name = "_id") val id: Long = 0,
    @ColumnInfo(name = "session_id") val sessionId: Long,
    @ColumnInfo(name = "kind") val kind: String,
    @ColumnInfo(name = "unix_ms") val unixMs: Long,
    @ColumnInfo(name = "detail") val detail: String,
    @ColumnInfo(name = "lat") val lat: Double? = null,
    @ColumnInfo(name = "lon") val lon: Double? = null,
    /** wolfPACK `detection_method_id` when kind is DETECTION_WIFI / DETECTION_BLE (legacy FLOCK_*). */
    @ColumnInfo(name = "detection_method_id") val detectionMethodId: Int? = null,
)

@Entity(
    tableName = "probe_observation",
    indices = [
        Index("src_mac"),
        Index("session_id"),
    ],
)
data class ProbeObservationEntity(
    @PrimaryKey(autoGenerate = true) @ColumnInfo(name = "_id") val id: Long = 0,
    @ColumnInfo(name = "session_id") val sessionId: Long,
    @ColumnInfo(name = "sniffer_id") val snifferId: Int,
    @ColumnInfo(name = "observed_unix_ms") val observedUnixMs: Long,
    @ColumnInfo(name = "lat") val lat: Double,
    @ColumnInfo(name = "lon") val lon: Double,
    @ColumnInfo(name = "altitude_m") val altitudeM: Int?,
    @ColumnInfo(name = "accuracy_m") val accuracyM: Float?,
    @ColumnInfo(name = "src_mac") val srcMac: String,
    @ColumnInfo(name = "ssid") val ssid: String,
    @ColumnInfo(name = "channel") val channel: Int,
    @ColumnInfo(name = "rssi") val rssi: Int,
)

@Dao
interface NetworkSummaryDao {
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun insertIgnore(summary: NetworkSummaryEntity): Long

    @Query(
        """
        UPDATE network_summary SET
            ssid = :ssid,
            last_rssi = :lastRssi,
            last_lat = :lastLat,
            last_lon = :lastLon,
            last_seen_unix_ms = :lastSeenUnixMs,
            observation_count = :observationCount,
            last_signal_kind = :lastSignalKind
        WHERE bssid = :bssid
        """,
    )
    suspend fun updateLastOnly(
        bssid: String,
        ssid: String,
        lastRssi: Int,
        lastLat: Double,
        lastLon: Double,
        lastSeenUnixMs: Long,
        observationCount: Int,
        lastSignalKind: String,
    )

    @Query("SELECT * FROM network_summary")
    suspend fun all(): List<NetworkSummaryEntity>

    @Query("SELECT * FROM network_summary")
    fun allFlow(): Flow<List<NetworkSummaryEntity>>

    @Query(
        """
        SELECT * FROM network_summary
        WHERE first_lat BETWEEN :south AND :north
          AND first_lon BETWEEN :west AND :east
        """,
    )
    fun inViewportFlow(
        south: Double,
        north: Double,
        west: Double,
        east: Double,
    ): Flow<List<NetworkSummaryEntity>>

    @Query("DELETE FROM network_summary")
    suspend fun deleteAll()
}

@Dao
interface SessionMapMarkerDao {
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun insert(marker: SessionMapMarkerEntity): Long

    @Query(
        """
        SELECT * FROM session_map_marker
        WHERE session_id = :sessionId
          AND lat BETWEEN :south AND :north
          AND lon BETWEEN :west AND :east
        """,
    )
    fun inViewportFlow(
        sessionId: Long,
        south: Double,
        north: Double,
        west: Double,
        east: Double,
    ): Flow<List<SessionMapMarkerEntity>>

    @Query("DELETE FROM session_map_marker")
    suspend fun deleteAll()
}

@Dao
interface DetectionDao {
    @Insert
    suspend fun insert(detection: DetectionEntity): Long

    @Query("SELECT * FROM detection WHERE session_id = :sessionId ORDER BY unix_ms")
    suspend fun forSession(sessionId: Long): List<DetectionEntity>

    @Query("SELECT * FROM detection WHERE session_id = :sessionId ORDER BY unix_ms")
    fun forSessionFlow(sessionId: Long): Flow<List<DetectionEntity>>

    @Query("SELECT * FROM detection ORDER BY unix_ms")
    fun allFlow(): Flow<List<DetectionEntity>>

    /** Latest detection row across all sessions (dashboard last-alert card). */
    @Query("SELECT * FROM detection ORDER BY unix_ms DESC LIMIT 1")
    suspend fun latest(): DetectionEntity?

    @Query("SELECT COUNT(*) FROM detection")
    suspend fun countAll(): Int

    @Query("DELETE FROM detection")
    suspend fun deleteAll()

    @Query(
        "SELECT * FROM detection " +
            "WHERE lat IS NOT NULL AND lat BETWEEN :south AND :north " +
            "AND lon IS NOT NULL AND lon BETWEEN :west AND :east",
    )
    fun detectionsInViewportFlow(
        south: Double,
        north: Double,
        west: Double,
        east: Double,
    ): Flow<List<DetectionEntity>>

    @Query("DELETE FROM detection WHERE UPPER(kind) LIKE '%GEOFENCE%'")
    suspend fun deleteGeofenceTransitions(): Int
}

@Dao
interface ObservationDao {
    @Insert
    suspend fun insert(observation: ObservationEntity): Long

    @Query("SELECT COUNT(*) FROM observation WHERE session_id = :sessionId")
    suspend fun countForSession(sessionId: Long): Int

    @Query(
        """
        SELECT COUNT(DISTINCT bssid) FROM observation
        WHERE session_id = :sessionId AND UPPER(signal_kind) = 'WIFI'
        """,
    )
    suspend fun countDistinctWifiBssidsForSession(sessionId: Long): Int

    @Query(
        """
        SELECT COUNT(DISTINCT bssid) FROM observation
        WHERE session_id = :sessionId AND UPPER(signal_kind) = 'BLE'
        """,
    )
    suspend fun countDistinctBleDevicesForSession(sessionId: Long): Int

    @Query(
        """
        SELECT COUNT(DISTINCT bssid) FROM observation
        WHERE session_id = :sessionId
        AND UPPER(signal_kind) IN ('GSM', 'WCDMA', 'LTE', 'NR', 'CDMA')
        """,
    )
    suspend fun countDistinctCellTowersForSession(sessionId: Long): Int

    @Query("SELECT COUNT(*) FROM observation WHERE bssid = :bssid")
    suspend fun countForBssid(bssid: String): Int

    @Query(
        """
        SELECT session_id FROM observation
        WHERE uploaded_wigle = 0
        GROUP BY session_id
        ORDER BY session_id
        LIMIT 1
        """,
    )
    suspend fun nextPendingWigleSessionId(): Long?

    @Query(
        """
        SELECT session_id FROM observation
        WHERE uploaded_wdgwars = 0
        GROUP BY session_id
        ORDER BY session_id
        LIMIT 1
        """,
    )
    suspend fun nextPendingWdgwarsSessionId(): Long?

    @Query(
        """
        SELECT * FROM observation
        WHERE session_id = :sessionId AND uploaded_wigle = 0 AND _id > :afterId
        ORDER BY _id
        LIMIT :limit
        """,
    )
    suspend fun pendingWigleUploadForSession(
        sessionId: Long,
        afterId: Long,
        limit: Int,
    ): List<ObservationEntity>

    @Query(
        """
        SELECT * FROM observation
        WHERE session_id = :sessionId AND uploaded_wdgwars = 0 AND _id > :afterId
        ORDER BY _id
        LIMIT :limit
        """,
    )
    suspend fun pendingWdgwarsUploadForSession(
        sessionId: Long,
        afterId: Long,
        limit: Int,
    ): List<ObservationEntity>

    @Query("SELECT COUNT(*) FROM observation WHERE session_id = :sessionId AND uploaded_wigle = 0")
    suspend fun pendingWigleCountForSession(sessionId: Long): Int

    @Query("SELECT COUNT(*) FROM observation WHERE session_id = :sessionId AND uploaded_wdgwars = 0")
    suspend fun pendingWdgwarsCountForSession(sessionId: Long): Int

    @Query("SELECT COUNT(*) FROM observation WHERE uploaded_wigle = 0")
    suspend fun pendingWigleUploadCountTotal(): Int

    @Query("SELECT COUNT(*) FROM observation WHERE uploaded_wdgwars = 0")
    suspend fun pendingWdgwarsUploadCountTotal(): Int

    @Query("UPDATE observation SET uploaded = 1, uploaded_wigle = 1 WHERE _id IN (:ids)")
    suspend fun markWigleUploadedIds(ids: List<Long>)

    @Query("UPDATE observation SET uploaded = 1, uploaded_wdgwars = 1 WHERE _id IN (:ids)")
    suspend fun markWdgwarsUploadedIds(ids: List<Long>)

    @Query("SELECT COUNT(*) FROM observation")
    suspend fun countAll(): Int

    @Query("DELETE FROM observation")
    suspend fun deleteAll()
}

@Dao
interface ProbeObservationDao {
    @Insert
    suspend fun insert(probe: ProbeObservationEntity): Long

    @Insert
    suspend fun insertAll(probes: List<ProbeObservationEntity>)

    @Query("SELECT COUNT(*) FROM probe_observation WHERE session_id = :sessionId")
    suspend fun countForSession(sessionId: Long): Int

    @Query("SELECT COUNT(*) FROM probe_observation")
    suspend fun countAll(): Int

    @Query("DELETE FROM probe_observation")
    suspend fun deleteAll()

    @Query(
        """
        SELECT * FROM probe_observation
        WHERE session_id = :sessionId
          AND lat BETWEEN :south AND :north
          AND lon BETWEEN :west AND :east
        """,
    )
    fun inViewportFlow(
        sessionId: Long,
        south: Double,
        north: Double,
        west: Double,
        east: Double,
    ): Flow<List<ProbeObservationEntity>>
}

@Dao
interface RouteDao {
    @Insert
    suspend fun insert(point: RoutePointEntity): Long

    @Query("SELECT * FROM route WHERE session_id = :sessionId ORDER BY _id")
    suspend fun forSession(sessionId: Long): List<RoutePointEntity>

    @Query("SELECT * FROM route WHERE session_id = :sessionId ORDER BY _id")
    fun forSessionFlow(sessionId: Long): Flow<List<RoutePointEntity>>

    @Query("SELECT * FROM route WHERE session_id = :sessionId ORDER BY _id DESC LIMIT 1")
    fun latestForSessionFlow(sessionId: Long): Flow<RoutePointEntity?>

    @Query("SELECT * FROM route ORDER BY session_id, unix_ms")
    fun allFlow(): Flow<List<RoutePointEntity>>

    /** Recent route samples across sessions for replay GPS hydration. */
    @Query(
        """
        SELECT * FROM route
        WHERE unix_ms >= :minUnixMs AND unix_ms <= :maxUnixMs
        ORDER BY unix_ms
        """,
    )
    suspend fun betweenUnixMs(minUnixMs: Long, maxUnixMs: Long): List<RoutePointEntity>

    @Query("DELETE FROM route")
    suspend fun deleteAll()
}

@Dao
interface SessionDao {
    @Insert
    suspend fun insert(session: SessionEntity): Long

    @Update
    suspend fun update(session: SessionEntity)

    @Query("SELECT * FROM session WHERE _id = :id")
    suspend fun get(id: Long): SessionEntity?

    @Query("SELECT * FROM session WHERE _id = :id")
    fun getFlow(id: Long): Flow<SessionEntity?>

    /** Most recent wardrive session (map fallback when no active session). */
    @Query("SELECT * FROM session ORDER BY _id DESC LIMIT 1")
    suspend fun getLatest(): SessionEntity?

    @Query("SELECT * FROM session WHERE ended_unix_ms IS NULL ORDER BY _id DESC")
    suspend fun getOpenSessions(): List<SessionEntity>

    @Query(
        """
        SELECT * FROM session
        WHERE ended_unix_ms IS NULL AND session_kind = :kind
        ORDER BY _id DESC LIMIT 1
        """,
    )
    suspend fun getLatestOpenOfKind(kind: String): SessionEntity?

    @Query(
        """
        UPDATE session SET ended_unix_ms = :endedMs
        WHERE ended_unix_ms IS NULL AND _id != :keepId
        """,
    )
    suspend fun closeOrphansExcept(keepId: Long, endedMs: Long)

    @Query("UPDATE session SET session_kind = :kind WHERE _id = :id")
    suspend fun updateSessionKind(id: Long, kind: String)

    @Query("UPDATE session SET gateway_mac = :mac WHERE _id = :id")
    suspend fun updateGatewayMac(id: Long, mac: String)

    @Query("SELECT COUNT(*) FROM session")
    suspend fun countAll(): Int

    @Query("UPDATE session SET ended_unix_ms = :endedMs WHERE ended_unix_ms IS NULL")
    suspend fun closeAllOpenSessions(endedMs: Long)

    @Query(
        """
        SELECT COUNT(*) FROM session
        WHERE (started_unix_ms / 1000) = (:startedUnixMs / 1000) AND _id != :sessionId
        """,
    )
    suspend fun countSessionsWithSameStartSecond(
        startedUnixMs: Long,
        sessionId: Long,
    ): Int

    @Query(
        """
        SELECT s.*, (SELECT COUNT(*) FROM observation o WHERE o.session_id = s._id) as obs_count
        FROM session s ORDER BY s.started_unix_ms DESC
        """,
    )
    suspend fun allSessionsWithCounts(): List<SessionWithCount>

    @Query("DELETE FROM session")
    suspend fun deleteAll()
}

@Dao
interface FleetRosterDao {
    @Upsert
    suspend fun upsert(row: FleetRosterEntity)

    @Query("SELECT * FROM fleet_roster WHERE adopted = 1 ORDER BY fleet_slot")
    fun adoptedFlow(): Flow<List<FleetRosterEntity>>

    @Query("SELECT * FROM fleet_roster WHERE adopted = 1 ORDER BY fleet_slot")
    suspend fun adopted(): List<FleetRosterEntity>

    @Query("SELECT * FROM fleet_roster WHERE adopt_pending = 1 AND adopted = 0 ORDER BY fleet_slot")
    fun adoptPendingFlow(): Flow<List<FleetRosterEntity>>

    @Query("SELECT * FROM fleet_roster WHERE adopt_pending = 1 AND adopted = 0 ORDER BY fleet_slot")
    suspend fun adoptPending(): List<FleetRosterEntity>

    /** Rows included in PolicySync roster: fully adopted or in-flight adopt. */
    @Query(
        """
        SELECT * FROM fleet_roster
        WHERE adopted = 1 OR adopt_pending = 1
        ORDER BY fleet_slot
        """,
    )
    suspend fun policyRoster(): List<FleetRosterEntity>

    @Query("SELECT * FROM fleet_roster WHERE sniffer_id = :snifferId")
    suspend fun get(snifferId: Int): FleetRosterEntity?

    @Delete
    suspend fun delete(row: FleetRosterEntity)

    @Query("DELETE FROM fleet_roster")
    suspend fun deleteAll(): Int
}

@Database(
    entities = [
        ObservationEntity::class,
        NetworkSummaryEntity::class,
        SessionMapMarkerEntity::class,
        RoutePointEntity::class,
        SessionEntity::class,
        DetectionEntity::class,
        FleetRosterEntity::class,
        ProbeObservationEntity::class,
    ],
    version = 14,
)
abstract class WardriverDatabase : RoomDatabase() {
    abstract fun observationDao(): ObservationDao
    abstract fun routeDao(): RouteDao
    abstract fun networkSummaryDao(): NetworkSummaryDao
    abstract fun sessionMapMarkerDao(): SessionMapMarkerDao
    abstract fun detectionDao(): DetectionDao
    abstract fun sessionDao(): SessionDao
    abstract fun fleetRosterDao(): FleetRosterDao
    abstract fun probeObservationDao(): ProbeObservationDao

    companion object {
        /** Room v1 -> v2: phone-owned `fleet_roster` for Sniffers screen. */
        val MIGRATION_1_2 =
            object : Migration(1, 2) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        """
                        CREATE TABLE IF NOT EXISTS `fleet_roster` (
                            `sniffer_id` INTEGER NOT NULL,
                            `mac` TEXT NOT NULL,
                            `fleet_slot` INTEGER NOT NULL,
                            `role` TEXT NOT NULL,
                            `band` TEXT NOT NULL,
                            `pinned_channel` INTEGER,
                            `adopted` INTEGER NOT NULL,
                            `last_seen_ms` INTEGER,
                            `cpu_load_pct` INTEGER,
                            `heap_avail_pct` INTEGER,
                            `reboot_count` INTEGER,
                            `state` INTEGER,
                            PRIMARY KEY(`sniffer_id`)
                        )
                        """.trimIndent(),
                    )
                }
            }

        /** Room v2 -> v3: sniffer-bound adopt (`adopt_pending`). */
        val MIGRATION_2_3 =
            object : Migration(2, 3) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `fleet_roster` ADD COLUMN `adopt_pending` INTEGER NOT NULL DEFAULT 0",
                    )
                }
            }

        /** Room v3 -> v4: `network_summary.last_signal_kind` for map WiFi/BLE filters. */
        val MIGRATION_3_4 =
            object : Migration(3, 4) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `network_summary` ADD COLUMN `last_signal_kind` TEXT NOT NULL DEFAULT 'WIFI'",
                    )
                    db.execSQL(
                        """
                        UPDATE network_summary SET last_signal_kind = COALESCE(
                            (SELECT signal_kind FROM observation o
                             WHERE o.bssid = network_summary.bssid
                             ORDER BY o.observed_unix_ms DESC LIMIT 1),
                            'WIFI')
                        """.trimIndent(),
                    )
                }
            }

        /** Room v4 -> v5: optional map coordinates on detection rows. */
        val MIGRATION_4_5 =
            object : Migration(4, 5) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL("ALTER TABLE `detection` ADD COLUMN `lat` REAL")
                    db.execSQL("ALTER TABLE `detection` ADD COLUMN `lon` REAL")
                }
            }

        /** Room v5 -> v6: session_kind for catch-up import sessions. */
        val MIGRATION_5_6 =
            object : Migration(5, 6) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `session` ADD COLUMN `session_kind` TEXT NOT NULL DEFAULT 'wardrive'",
                    )
                }
            }

        /** Room v6 -> v7: per-destination upload flags on observations. */
        val MIGRATION_6_7 =
            object : Migration(6, 7) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `observation` ADD COLUMN `uploaded_wigle` INTEGER NOT NULL DEFAULT 0",
                    )
                    db.execSQL(
                        "ALTER TABLE `observation` ADD COLUMN `uploaded_wdgwars` INTEGER NOT NULL DEFAULT 0",
                    )
                    db.execSQL(
                        """
                        UPDATE observation
                        SET uploaded_wigle = uploaded, uploaded_wdgwars = uploaded
                        WHERE uploaded = 1
                        """.trimIndent(),
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_observation_uploaded_wigle` ON `observation` (`uploaded_wigle`)",
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_observation_uploaded_wdgwars` ON `observation` (`uploaded_wdgwars`)",
                    )
                }
            }

        /** Room v7 -> v8: composite indexes for session-scoped upload queries. */
        val MIGRATION_7_8 =
            object : Migration(7, 8) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_observation_session_id_uploaded_wigle` " +
                            "ON `observation` (`session_id`, `uploaded_wigle`)",
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_observation_session_id_uploaded_wdgwars` " +
                            "ON `observation` (`session_id`, `uploaded_wdgwars`)",
                    )
                }
            }

        /** Room v8 -> v9: first-seen map markers + network_summary first_* columns. */
        val MIGRATION_8_9 =
            object : Migration(8, 9) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `network_summary` ADD COLUMN `first_lat` REAL NOT NULL DEFAULT 0",
                    )
                    db.execSQL(
                        "ALTER TABLE `network_summary` ADD COLUMN `first_lon` REAL NOT NULL DEFAULT 0",
                    )
                    db.execSQL(
                        "ALTER TABLE `network_summary` ADD COLUMN `first_seen_unix_ms` INTEGER NOT NULL DEFAULT 0",
                    )
                    db.execSQL(
                        """
                        UPDATE network_summary SET
                            first_lat = COALESCE(
                                (SELECT lat FROM observation o
                                 WHERE o.bssid = network_summary.bssid
                                 ORDER BY o.observed_unix_ms ASC LIMIT 1),
                                last_lat),
                            first_lon = COALESCE(
                                (SELECT lon FROM observation o
                                 WHERE o.bssid = network_summary.bssid
                                 ORDER BY o.observed_unix_ms ASC LIMIT 1),
                                last_lon),
                            first_seen_unix_ms = COALESCE(
                                (SELECT observed_unix_ms FROM observation o
                                 WHERE o.bssid = network_summary.bssid
                                 ORDER BY o.observed_unix_ms ASC LIMIT 1),
                                last_seen_unix_ms)
                        """.trimIndent(),
                    )
                    db.execSQL(
                        """
                        CREATE TABLE IF NOT EXISTS `session_map_marker` (
                            `session_id` INTEGER NOT NULL,
                            `bssid` TEXT NOT NULL,
                            `lat` REAL NOT NULL,
                            `lon` REAL NOT NULL,
                            `ssid` TEXT NOT NULL,
                            `rssi` INTEGER NOT NULL,
                            `signal_kind` TEXT NOT NULL DEFAULT 'WIFI',
                            `observed_unix_ms` INTEGER NOT NULL,
                            PRIMARY KEY(`session_id`, `bssid`)
                        )
                        """.trimIndent(),
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_session_map_marker_session_id_lat_lon` " +
                            "ON `session_map_marker` (`session_id`, `lat`, `lon`)",
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_network_summary_first_lat_first_lon` " +
                            "ON `network_summary` (`first_lat`, `first_lon`)",
                    )
                    db.execSQL(
                        """
                        INSERT OR IGNORE INTO session_map_marker (
                            session_id, bssid, lat, lon, ssid, rssi, signal_kind, observed_unix_ms
                        )
                        SELECT o.session_id, o.bssid, o.lat, o.lon, o.ssid, o.rssi, o.signal_kind,
                               o.observed_unix_ms
                        FROM observation o
                        WHERE o._id = (
                            SELECT _id FROM observation
                            WHERE session_id = o.session_id AND bssid = o.bssid
                            ORDER BY observed_unix_ms ASC LIMIT 1
                        )
                        """.trimIndent(),
                    )
                }
            }

        /** Room v9 -> v10: flock `detection_method_id` on detection rows. */
        val MIGRATION_9_10 =
            object : Migration(9, 10) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `detection` ADD COLUMN `detection_method_id` INTEGER",
                    )
                }
            }

        /** Room v10 -> v11: WiGLE RCOIs + BLE AuthMode legend for CSV export. */
        val MIGRATION_10_11 =
            object : Migration(10, 11) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "ALTER TABLE `observation` ADD COLUMN `rcois` TEXT NOT NULL DEFAULT ''",
                    )
                    db.execSQL(
                        "ALTER TABLE `observation` ADD COLUMN `ble_capabilities` TEXT NOT NULL DEFAULT ''",
                    )
                }
            }

        /** Room v11 -> v12: spatial index on detection(lat, lon) for viewport-bounded AllDevices query. */
        val MIGRATION_11_12 =
            object : Migration(11, 12) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS index_detection_lat_lon ON detection (lat, lon)",
                    )
                }
            }

        /** Room v12 -> v13: `probe_observation` table for probe request client logging. */
        val MIGRATION_12_13 =
            object : Migration(12, 13) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        """
                        CREATE TABLE IF NOT EXISTS `probe_observation` (
                            `_id` INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                            `session_id` INTEGER NOT NULL,
                            `sniffer_id` INTEGER NOT NULL,
                            `observed_unix_ms` INTEGER NOT NULL,
                            `lat` REAL NOT NULL,
                            `lon` REAL NOT NULL,
                            `altitude_m` INTEGER,
                            `accuracy_m` REAL,
                            `src_mac` TEXT NOT NULL,
                            `ssid` TEXT NOT NULL,
                            `channel` INTEGER NOT NULL,
                            `rssi` INTEGER NOT NULL
                        )
                        """.trimIndent(),
                    )
                    db.execSQL("CREATE INDEX IF NOT EXISTS `index_probe_observation_src_mac` ON `probe_observation` (`src_mac`)")
                    db.execSQL("CREATE INDEX IF NOT EXISTS `index_probe_observation_session_id` ON `probe_observation` (`session_id`)")
                }
            }

        /**
         * Room v13 -> v14: rebuild `probe_observation`.
         *
         * MIGRATION_12_13 originally created `_id` without `NOT NULL`; Room expects
         * `NOT NULL` for the autoGenerate non-null primary key, so schema validation
         * failed on every DB open (startup crash) for installs already at v13.
         * Rebuild from surviving rows; correct for both pre-fix and post-fix v13 tables.
         */
        val MIGRATION_13_14 =
            object : Migration(13, 14) {
                override fun migrate(db: SupportSQLiteDatabase) {
                    db.execSQL("DROP TABLE IF EXISTS `_new_probe_observation`")
                    db.execSQL(
                        """
                        CREATE TABLE `_new_probe_observation` (
                            `_id` INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                            `session_id` INTEGER NOT NULL,
                            `sniffer_id` INTEGER NOT NULL,
                            `observed_unix_ms` INTEGER NOT NULL,
                            `lat` REAL NOT NULL,
                            `lon` REAL NOT NULL,
                            `altitude_m` INTEGER,
                            `accuracy_m` REAL,
                            `src_mac` TEXT NOT NULL,
                            `ssid` TEXT NOT NULL,
                            `channel` INTEGER NOT NULL,
                            `rssi` INTEGER NOT NULL
                        )
                        """.trimIndent(),
                    )
                    db.execSQL(
                        """
                        INSERT INTO `_new_probe_observation`
                            (`_id`, `session_id`, `sniffer_id`, `observed_unix_ms`, `lat`, `lon`,
                             `altitude_m`, `accuracy_m`, `src_mac`, `ssid`, `channel`, `rssi`)
                        SELECT `_id`, `session_id`, `sniffer_id`, `observed_unix_ms`, `lat`, `lon`,
                               `altitude_m`, `accuracy_m`, `src_mac`, `ssid`, `channel`, `rssi`
                        FROM `probe_observation`
                        """.trimIndent(),
                    )
                    db.execSQL("DROP TABLE `probe_observation`")
                    db.execSQL("ALTER TABLE `_new_probe_observation` RENAME TO `probe_observation`")
                    db.execSQL("CREATE INDEX IF NOT EXISTS `index_probe_observation_src_mac` ON `probe_observation` (`src_mac`)")
                    db.execSQL("CREATE INDEX IF NOT EXISTS `index_probe_observation_session_id` ON `probe_observation` (`session_id`)")
                }
            }

        @Volatile
        private var instance: WardriverDatabase? = null

        fun getInstance(context: Context): WardriverDatabase =
            instance ?: synchronized(this) {
                instance ?: Room.databaseBuilder(
                    context.applicationContext,
                    WardriverDatabase::class.java,
                    "wardriver.db",
                )
                    .addMigrations(
                        MIGRATION_1_2,
                        MIGRATION_2_3,
                        MIGRATION_3_4,
                        MIGRATION_4_5,
                        MIGRATION_5_6,
                        MIGRATION_6_7,
                        MIGRATION_7_8,
                        MIGRATION_8_9,
                        MIGRATION_9_10,
                        MIGRATION_10_11,
                        MIGRATION_11_12,
                        MIGRATION_12_13,
                        MIGRATION_13_14,
                    )
                    .fallbackToDestructiveMigration()
                    .build()
                    .also { instance = it }
            }
    }
}
