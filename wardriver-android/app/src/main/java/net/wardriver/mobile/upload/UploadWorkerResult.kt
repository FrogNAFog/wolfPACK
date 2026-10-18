package net.wardriver.mobile.upload

import androidx.work.ListenableWorker.Result
import androidx.work.workDataOf

const val KEY_BATCH = "batch"
const val KEY_ROWS_UPLOADED = "rows_uploaded"
const val KEY_DEST = "dest"
const val KEY_ERROR = "error"
const val KEY_BATCHES = "batches"

const val KEY_ROWS_WIGLE = "rows_uploaded_wigle"
const val KEY_ROWS_WDGWARS = "rows_uploaded_wdgwars"
const val KEY_BATCHES_WIGLE = "batches_wigle"
const val KEY_BATCHES_WDGWARS = "batches_wdgwars"
const val KEY_ERROR_WIGLE = "error_wigle"
const val KEY_ERROR_WDGWARS = "error_wdgwars"
const val KEY_SESSION = "session"
const val KEY_FILENAME = "filename"

const val DEST_WIGLE = "wigle"
const val DEST_WDGWARS = "wdgwars"

fun resultFromSerializedRun(run: SerializedUploadRunResult): Result {
    val wigle = run.resultFor(DEST_WIGLE)
    val wdg = run.resultFor(DEST_WDGWARS)
    val firstError =
        if (run.hasAnyError) {
            run.destinations.firstNotNullOfOrNull { dest ->
                dest.error?.let { "${dest.destId}: $it" }
            } ?: "upload failed"
        } else {
            ""
        }
    val data =
        workDataOf(
            KEY_ROWS_WIGLE to (wigle?.rowsUploaded ?: 0),
            KEY_ROWS_WDGWARS to (wdg?.rowsUploaded ?: 0),
            KEY_BATCHES_WIGLE to (wigle?.batches ?: 0),
            KEY_BATCHES_WDGWARS to (wdg?.batches ?: 0),
            KEY_ERROR_WIGLE to (wigle?.error ?: ""),
            KEY_ERROR_WDGWARS to (wdg?.error ?: ""),
            KEY_ROWS_UPLOADED to run.destinations.sumOf { it.rowsUploaded },
            KEY_BATCHES to run.destinations.sumOf { it.batches },
            KEY_ERROR to firstError,
        )

    return if (run.hasAnyError) Result.failure(data) else Result.success(data)
}
