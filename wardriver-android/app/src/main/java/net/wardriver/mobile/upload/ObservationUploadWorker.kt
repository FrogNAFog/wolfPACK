package net.wardriver.mobile.upload

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import net.wardriver.mobile.db.WardriverDatabase

/**
 * User-initiated upload: closes the active session, POSTs each pending session in 5 MiB
 * WiGLE CSV chunks to each configured destination serially, then resumes wardrive if needed.
 */
class ObservationUploadWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {
    override suspend fun doWork(): Result {
        val prepCompleted = UploadCoordinator.awaitPrepareForUpload(applicationContext)
        if (prepCompleted == null) {
            UploadCoordinator.uploadInProgress = false
            return Result.failure(workDataOf(KEY_ERROR to "Upload prep timed out"))
        }

        val startMs = System.currentTimeMillis()
        try {
            val prefs = UploadPreferences(applicationContext)
            val destFilter = inputData.getString(KEY_DEST_FILTER) ?: DEST_FILTER_ALL
            val db = WardriverDatabase.getInstance(applicationContext)
            val client = uploadHttpClient()
            val dao = db.observationDao()

            val destinations =
                configuredUploadDestinations(
                    client = client,
                    dao = dao,
                    prefs = prefs,
                    destFilter = destFilter,
                )
            if (destinations.isEmpty()) {
                return Result.failure(workDataOf(KEY_ERROR to "No upload destinations configured"))
            }

            val run =
                runSerializedUploadLoop(
                    db = db,
                    destinations = destinations,
                    uploadPrefs = prefs,
                    onProgress = { destId, sessionId, fileName, batch, rows ->
                        setProgress(
                            workDataOf(
                                KEY_BATCH to batch,
                                KEY_ROWS_UPLOADED to rows,
                                KEY_DEST to destId,
                                KEY_SESSION to sessionId,
                                KEY_FILENAME to fileName,
                            ),
                        )
                    },
                )

            return resultFromSerializedRun(run)
        } finally {
            logUploadRunDuration(System.currentTimeMillis() - startMs)
            UploadCoordinator.requestResumeAfterUpload(applicationContext)
        }
    }

    companion object {
        const val WORK_NAME = "wardriver_upload"
    }
}
