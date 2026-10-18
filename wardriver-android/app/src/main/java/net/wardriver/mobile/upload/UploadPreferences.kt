package net.wardriver.mobile.upload

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.data.AppPreferences

/** Per-(destination, session) chunk counter for stable multipart filenames across retries. */
interface UploadChunkIndexStore {
    fun getChunkIndex(dest: UploadDest, sessionId: Long): Int

    fun setChunkIndex(dest: UploadDest, sessionId: Long, index: Int)

    fun clearChunkIndex(dest: UploadDest, sessionId: Long)
}

/**
 * Upload cursors + API credentials.
 * Credentials live in DataStore ([AppPreferences]); per-destination markers stay in SharedPreferences.
 */
class UploadPreferences(context: Context) : UploadChunkIndexStore {
    private val appContext = context.applicationContext
    private val appPreferences = AppPreferences(appContext)
    private val prefs: SharedPreferences =
        appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /** Last observation `_id` included in a successful WiGLE upload batch. */
    var dbMarkerWigle: Long
        get() = prefs.getLong(KEY_DB_MARKER_WIGLE, 0L)
        set(value) = prefs.edit().putLong(KEY_DB_MARKER_WIGLE, value).apply()

    /** Last observation `_id` included in a successful wdgwars upload batch. */
    var dbMarkerWdgwars: Long
        get() = prefs.getLong(KEY_DB_MARKER_WDGWARS, 0L)
        set(value) = prefs.edit().putLong(KEY_DB_MARKER_WDGWARS, value).apply()

    /** Legacy single marker (migrated to per-destination keys on first read). */
    @Deprecated("Use dbMarkerWigle / dbMarkerWdgwars")
    var dbMarker: Long
        get() = dbMarkerWigle
        set(value) {
            dbMarkerWigle = value
            dbMarkerWdgwars = value
        }

    fun ensureMarkersMigrated() {
        if (!prefs.contains(KEY_DB_MARKER_WIGLE) && prefs.contains(KEY_DB_MARKER)) {
            val legacy = prefs.getLong(KEY_DB_MARKER, 0L)
            prefs.edit()
                .putLong(KEY_DB_MARKER_WIGLE, legacy)
                .putLong(KEY_DB_MARKER_WDGWARS, legacy)
                .apply()
        }
    }

    fun clearMarkers() {
        prefs.edit()
            .putLong(KEY_DB_MARKER_WIGLE, 0L)
            .putLong(KEY_DB_MARKER_WDGWARS, 0L)
            .putLong(KEY_DB_MARKER, 0L)
            .apply()
    }

    /** Next chunk index to POST for this session (1-based; chunk 1 has no filename suffix). */
    override fun getChunkIndex(dest: UploadDest, sessionId: Long): Int =
        prefs.getInt(chunkKey(dest, sessionId), 1)

    override fun setChunkIndex(dest: UploadDest, sessionId: Long, index: Int) {
        prefs.edit().putInt(chunkKey(dest, sessionId), index).apply()
    }

    override fun clearChunkIndex(dest: UploadDest, sessionId: Long) {
        prefs.edit().remove(chunkKey(dest, sessionId)).apply()
    }

    private fun chunkKey(dest: UploadDest, sessionId: Long): String =
        when (dest) {
            UploadDest.WIGLE -> "${KEY_CHUNK_WIGLE}_$sessionId"
            UploadDest.WDGWARS -> "${KEY_CHUNK_WDGWARS}_$sessionId"
        }

    /** WiGLE API display name (DataStore). */
    suspend fun wigleApiName(): String? =
        appPreferences.wigleApiNameFlow.first().ifBlank { null }

    /** WiGLE API token (DataStore). */
    suspend fun wigleApiToken(): String? =
        appPreferences.wigleApiTokenFlow.first().ifBlank { null }

    /** wdgwars API key (DataStore). */
    suspend fun wdgwarsApiKey(): String? =
        appPreferences.wdgwarsApiKeyFlow.first().ifBlank { null }

    suspend fun isWigleConfigured(): Boolean =
        !wigleApiName().isNullOrBlank() && !wigleApiToken().isNullOrBlank()

    suspend fun isWdgwarsConfigured(): Boolean = !wdgwarsApiKey().isNullOrBlank()

    companion object {
        const val PREFS_NAME = "wardriver_upload"
        const val KEY_DB_MARKER = "db_marker"
        const val KEY_DB_MARKER_WIGLE = "db_marker_wigle"
        const val KEY_DB_MARKER_WDGWARS = "db_marker_wdgwars"
        const val KEY_CHUNK_WIGLE = "upload_chunk_wigle"
        const val KEY_CHUNK_WDGWARS = "upload_chunk_wdgwars"
    }
}
