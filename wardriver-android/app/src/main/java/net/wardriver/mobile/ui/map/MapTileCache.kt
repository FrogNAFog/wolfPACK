package net.wardriver.mobile.ui.map

import android.content.Context
import org.osmdroid.config.Configuration
import java.io.File

/**
 * One-shot cleanup of legacy osmdroid tile caches.
 *
 * Basemap switches rely on distinct tile-source names; old tiles age out under osmdroid LRU.
 * Never use [org.osmdroid.tileprovider.cachemanager.CacheManager.cleanAreaAsync] with a
 * world bounding box — that iterates theoretical tile indices for zoom 0–20.
 */
object MapTileCache {
    private const val PREFS = "map_tile_cache"
    private const val KEY_LEGACY_MAPNIK_PURGED = "legacy_mapnik_purged_v1"

    /** Delete leftover MAPNIK tiles once per install (pre-basemap-preference builds). */
    fun purgeLegacyMapnikOnce(context: Context) {
        val app = context.applicationContext
        val prefs = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (prefs.getBoolean(KEY_LEGACY_MAPNIK_PURGED, false)) return
        deleteTileSourceDir(app, PackTileSources.legacyMapnikName)
        prefs.edit().putBoolean(KEY_LEGACY_MAPNIK_PURGED, true).apply()
    }

    private fun deleteTileSourceDir(context: Context, sourceName: String) {
        try {
            val root = Configuration.getInstance().osmdroidTileCache
                ?: File(context.filesDir, "osmdroid/tiles")
            val dir = File(root, sourceName)
            if (dir.isDirectory) {
                dir.deleteRecursively()
            }
        } catch (_: Exception) {
            // Best-effort; tiles will age out under LRU.
        }
    }
}
