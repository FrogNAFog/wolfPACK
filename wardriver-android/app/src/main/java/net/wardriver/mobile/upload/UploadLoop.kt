package net.wardriver.mobile.upload

import android.util.Log
import androidx.room.withTransaction
import net.wardriver.mobile.db.SessionDao
import net.wardriver.mobile.db.WardriverDatabase

data class DestinationUploadResult(
    val destId: String,
    val rowsUploaded: Int,
    val batches: Int,
    val error: String? = null,
)

data class SerializedUploadRunResult(
    val destinations: List<DestinationUploadResult>,
) {
    val hasAnyError: Boolean
        get() = destinations.any { it.error != null }

    fun resultFor(destId: String): DestinationUploadResult? =
        destinations.firstOrNull { it.destId == destId }
}

private const val MARK_FAILED_MSG =
    "upload accepted by server but local mark failed — do not retry without support"

private const val TAG = "wardriver-upload"

/** Run each destination phase serially; sessions and 5 MiB chunks never cross session boundaries. */
suspend fun runSerializedUploadLoop(
    destinations: List<UploadDestinationConfig>,
    sessionDao: SessionDao,
    uploadPrefs: UploadChunkIndexStore,
    markBatch: suspend (UploadDest, List<Long>) -> Unit,
    onProgress: suspend (destId: String, sessionId: Long, fileName: String, batch: Int, rowsSoFar: Int) -> Unit,
    headers: String = WigleCsvBuilder.headers(),
    lineFormatter: (net.wardriver.mobile.db.ObservationEntity) -> String? = ::formatUploadCsvLine,
): SerializedUploadRunResult {
    val results = mutableListOf<DestinationUploadResult>()

    for (config in destinations) {
        var totalRows = 0
        var batchCount = 0
        var destError: String? = null

        while (destError == null) {
            val sessionId = config.nextPendingSessionId() ?: break
            val session =
                sessionDao.get(sessionId)
                    ?: break
            val useSessionInfix =
                sessionDao.countSessionsWithSameStartSecond(session.startedUnixMs, session.id) > 0

            while (config.pendingCountForSession(sessionId) > 0 && destError == null) {
                val chunkIndex = uploadPrefs.getChunkIndex(config.dest, sessionId)
                val batch =
                    assembleBatchForSession(
                        sessionId = sessionId,
                        headers = headers,
                        maxBytes = UPLOAD_CHUNK_BYTES,
                        fetchChunk = config.fetchSessionChunk,
                        lineFormatter = lineFormatter,
                    )
                if (batch.rowCount == 0) {
                    break
                }

                batchCount += 1
                val fileName = uploadCsvFileName(session, chunkIndex, useSessionInfix)
                val postResult =
                    config.postCsv(batch.csvText.toByteArray(Charsets.UTF_8), fileName)
                if (postResult.isFailure) {
                    destError = postResult.exceptionOrNull()?.message ?: "upload failed"
                    batchCount -= 1
                    break
                }

                try {
                    markBatch(config.dest, batch.uploadedIds)
                    uploadPrefs.setChunkIndex(config.dest, sessionId, chunkIndex + 1)
                } catch (e: Exception) {
                    logUploadMarkFailure(e)
                    destError = MARK_FAILED_MSG
                    batchCount -= 1
                    break
                }

                totalRows += batch.rowCount
                onProgress(config.destId, sessionId, fileName, batchCount, totalRows)

                if (config.pendingCountForSession(sessionId) == 0) {
                    uploadPrefs.clearChunkIndex(config.dest, sessionId)
                }
            }

            if (destError != null) {
                break
            }
        }

        results +=
            DestinationUploadResult(
                destId = config.destId,
                rowsUploaded = totalRows,
                batches = batchCount,
                error = destError,
            )
    }

    return SerializedUploadRunResult(results)
}

suspend fun runSerializedUploadLoop(
    db: WardriverDatabase,
    destinations: List<UploadDestinationConfig>,
    uploadPrefs: UploadChunkIndexStore,
    onProgress: suspend (destId: String, sessionId: Long, fileName: String, batch: Int, rowsSoFar: Int) -> Unit,
): SerializedUploadRunResult =
    runSerializedUploadLoop(
        destinations = destinations,
        sessionDao = db.sessionDao(),
        uploadPrefs = uploadPrefs,
        markBatch = { dest, ids ->
            db.withTransaction {
                db.observationDao().markUploadedIdsChunked(ids, dest)
            }
        },
        onProgress = onProgress,
    )

internal fun logUploadRunDuration(wallMs: Long) {
    Log.i(TAG, "upload run wall_ms=$wallMs")
}
