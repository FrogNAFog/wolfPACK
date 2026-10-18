package net.wardriver.mobile.upload

import net.wardriver.mobile.db.ObservationDao
import net.wardriver.mobile.db.ObservationEntity
import net.wardriver.mobile.db.SessionDao
import okhttp3.OkHttpClient

enum class UploadDest {
    WIGLE,
    WDGWARS,
}

data class UploadDestinationConfig(
    val dest: UploadDest,
    val destId: String,
    val nextPendingSessionId: suspend () -> Long?,
    val pendingCountForSession: suspend (Long) -> Int,
    val fetchSessionChunk: suspend (sessionId: Long, afterId: Long, limit: Int) -> List<ObservationEntity>,
    val postCsv: suspend (ByteArray, String) -> Result<Unit>,
)

const val KEY_DEST_FILTER = "dest_filter"
const val DEST_FILTER_ALL = "all"

suspend fun configuredUploadDestinations(
    client: OkHttpClient,
    dao: ObservationDao,
    prefs: UploadPreferences,
    destFilter: String,
): List<UploadDestinationConfig> {
    val destinations = mutableListOf<UploadDestinationConfig>()

    val includeWdg =
        (destFilter == DEST_FILTER_ALL || destFilter == DEST_WDGWARS) && prefs.isWdgwarsConfigured()
    val includeWigle =
        (destFilter == DEST_FILTER_ALL || destFilter == DEST_WIGLE) && prefs.isWigleConfigured()

    if (includeWdg) {
        val apiKey = prefs.wdgwarsApiKey()!!
        destinations +=
            UploadDestinationConfig(
                dest = UploadDest.WDGWARS,
                destId = DEST_WDGWARS,
                nextPendingSessionId = { dao.nextPendingWdgwarsSessionId() },
                pendingCountForSession = { sid -> dao.pendingWdgwarsCountForSession(sid) },
                fetchSessionChunk = { sid, afterId, limit ->
                    dao.pendingWdgwarsUploadForSession(sid, afterId, limit)
                },
                postCsv = { bytes, fileName -> postWdgCsv(client, apiKey, bytes, fileName) },
            )
    }

    if (includeWigle) {
        val apiName = prefs.wigleApiName()!!
        val apiToken = prefs.wigleApiToken()!!
        destinations +=
            UploadDestinationConfig(
                dest = UploadDest.WIGLE,
                destId = DEST_WIGLE,
                nextPendingSessionId = { dao.nextPendingWigleSessionId() },
                pendingCountForSession = { sid -> dao.pendingWigleCountForSession(sid) },
                fetchSessionChunk = { sid, afterId, limit ->
                    dao.pendingWigleUploadForSession(sid, afterId, limit)
                },
                postCsv = { bytes, fileName ->
                    postWigleCsv(client, apiName, apiToken, bytes, fileName)
                },
            )
    }

    return destinations
}
