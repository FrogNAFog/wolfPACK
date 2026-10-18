package net.wardriver.mobile.upload

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.collectAsState
import androidx.work.WorkInfo
import androidx.work.WorkManager

data class DestinationUploadState(
    val dest: String,
    val running: Boolean = false,
    val batch: Int = 0,
    val rowsSoFar: Int = 0,
    val finishedRows: Int? = null,
    val finishedBatches: Int? = null,
    val error: String? = null,
    val workState: WorkInfo.State? = null,
)

data class UploadUiState(
    val wigle: DestinationUploadState = DestinationUploadState(DEST_WIGLE),
    val wdgwars: DestinationUploadState = DestinationUploadState(DEST_WDGWARS),
) {
    val anyRunning: Boolean
        get() = wigle.running || wdgwars.running
}

@Composable
fun rememberUploadUiState(context: Context): UploadUiState {
    val workManager = WorkManager.getInstance(context)
    val uploadInfos by workManager
        .getWorkInfosForUniqueWorkFlow(ObservationUploadWorker.WORK_NAME)
        .collectAsState(initial = emptyList())

    val info = uploadInfos.firstOrNull()
    return UploadUiState(
        wigle = parseDestinationState(DEST_WIGLE, info),
        wdgwars = parseDestinationState(DEST_WDGWARS, info),
    )
}

private fun parseDestinationState(
    dest: String,
    info: WorkInfo?,
): DestinationUploadState {
    if (info == null) {
        return DestinationUploadState(dest)
    }

    val state = info.state
    val progress = info.progress
    val output = info.outputData
    val rowsKey = if (dest == DEST_WIGLE) KEY_ROWS_WIGLE else KEY_ROWS_WDGWARS
    val batchesKey = if (dest == DEST_WIGLE) KEY_BATCHES_WIGLE else KEY_BATCHES_WDGWARS
    val errorKey = if (dest == DEST_WIGLE) KEY_ERROR_WIGLE else KEY_ERROR_WDGWARS
    val activeDest = progress.getString(KEY_DEST)

    return when (state) {
        WorkInfo.State.RUNNING, WorkInfo.State.ENQUEUED -> {
            val isActivePhase = state == WorkInfo.State.ENQUEUED || activeDest == dest
            DestinationUploadState(
                dest = dest,
                running = isActivePhase,
                batch = if (isActivePhase) progress.getInt(KEY_BATCH, 0) else 0,
                rowsSoFar = if (isActivePhase) progress.getInt(KEY_ROWS_UPLOADED, 0) else 0,
                workState = state,
            )
        }
        WorkInfo.State.SUCCEEDED -> {
            DestinationUploadState(
                dest = dest,
                finishedRows = output.getInt(rowsKey, 0),
                finishedBatches = output.getInt(batchesKey, 0),
                workState = state,
            )
        }
        WorkInfo.State.FAILED, WorkInfo.State.CANCELLED -> {
            val error = output.getString(errorKey)?.takeIf { it.isNotBlank() }
            DestinationUploadState(
                dest = dest,
                error = error ?: output.getString(KEY_ERROR),
                finishedRows = output.getInt(rowsKey, 0),
                finishedBatches = output.getInt(batchesKey, 0),
                workState = state,
            )
        }
        else -> DestinationUploadState(dest = dest, workState = state)
    }
}

fun formatUploadStatusLine(state: DestinationUploadState, label: String): String? {
    return when {
        state.running -> {
            val batch = state.batch.coerceAtLeast(1)
            val rows = state.rowsSoFar
            "Uploading to $label… batch $batch ($rows rows so far)"
        }
        state.error != null -> {
            val rows = state.finishedRows ?: 0
            if (rows > 0) {
                "$label upload failed after $rows rows (${state.error})"
            } else {
                "$label upload failed (${state.error})"
            }
        }
        state.finishedRows != null && state.workState == WorkInfo.State.SUCCEEDED -> {
            val rows = state.finishedRows
            val batches = state.finishedBatches ?: 0
            when {
                rows == 0 -> "$label: nothing to upload"
                batches > 1 -> "$label: uploaded $rows rows in $batches batches"
                else -> "$label: uploaded $rows rows"
            }
        }
        else -> null
    }
}

suspend fun loadPendingUploadCounts(context: Context): PendingUploadCounts {
    val dao = net.wardriver.mobile.db.WardriverDatabase.getInstance(context).observationDao()
    return PendingUploadCounts(
        wiglePending = dao.pendingWigleUploadCountTotal(),
        wdgPending = dao.pendingWdgwarsUploadCountTotal(),
    )
}

data class PendingUploadCounts(
    val wiglePending: Int,
    val wdgPending: Int,
)
