package net.wardriver.mobile.upload

import android.content.Context
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager

/** Enqueue user-initiated upload workers (notification UPLOAD + settings). */
object UploadScheduler {
    fun scheduleAll(context: Context) {
        enqueue(context.applicationContext, DEST_FILTER_ALL)
    }

    private fun enqueue(
        context: Context,
        destFilter: String,
    ) {
        val input =
            Data.Builder()
                .putString(KEY_DEST_FILTER, destFilter)
                .build()
        val req =
            OneTimeWorkRequestBuilder<ObservationUploadWorker>()
                .setInputData(input)
                .build()
        WorkManager.getInstance(context)
            .enqueueUniqueWork(
                ObservationUploadWorker.WORK_NAME,
                ExistingWorkPolicy.REPLACE,
                req,
            )
    }
}
