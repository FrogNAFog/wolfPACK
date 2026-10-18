package net.wardriver.mobile.data

import android.content.Context
import androidx.work.WorkManager
import androidx.room.withTransaction
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.ingest.ObservationIngestStats
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.upload.ObservationUploadWorker
import net.wardriver.mobile.upload.UploadPreferences

/** Clears local wardrive capture tables; preserves fleet roster. */
class ObservationDataRepository(
    private val db: WardriverDatabase,
) {
    sealed class ResetResult {
        data class Ok(val wardriveWasStopped: Boolean) : ResetResult()

        data object WardriveStopTimeout : ResetResult()
    }

    data class LocalDataCounts(
        val observations: Int,
        val sessions: Int,
        val detections: Int,
    )

    suspend fun loadCounts(): LocalDataCounts =
        withContext(Dispatchers.IO) {
            LocalDataCounts(
                observations = db.observationDao().countAll(),
                sessions = db.sessionDao().countAll(),
                detections = db.detectionDao().countAll(),
            )
        }

    suspend fun clearAllObservationData(context: Context): ResetResult {
        val wardriveWasActive = WardriveSessionService.sessionIdFlow.value != null
        if (wardriveWasActive) {
            WardriveSessionService.stop(context)
            val ended = WardriveSessionService.awaitSessionEnded(timeoutMs = 10_000)
            if (!ended) return ResetResult.WardriveStopTimeout
        }
        withContext(Dispatchers.IO) {
            db.withTransaction {
                db.observationDao().deleteAll()
                db.networkSummaryDao().deleteAll()
                db.sessionMapMarkerDao().deleteAll()
                db.routeDao().deleteAll()
                db.sessionDao().deleteAll()
                db.detectionDao().deleteAll()
            }
            UploadPreferences(context).clearMarkers()
            ObservationIngestStats.reset()
            val wm = WorkManager.getInstance(context)
            wm.cancelUniqueWork(ObservationUploadWorker.WORK_NAME)
        }
        return ResetResult.Ok(wardriveWasStopped = wardriveWasActive)
    }
}
