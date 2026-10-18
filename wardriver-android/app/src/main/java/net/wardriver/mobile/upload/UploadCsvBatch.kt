package net.wardriver.mobile.upload

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.db.ObservationEntity
import net.wardriver.mobile.ingest.CellTowerCollector
import org.json.JSONObject

/** Per-upload POST body cap (5 MiB binary). */
const val UPLOAD_CHUNK_BYTES: Long = 5L * 1024L * 1024L

@Deprecated("Use UPLOAD_CHUNK_BYTES", ReplaceWith("UPLOAD_CHUNK_BYTES"))
const val MAX_CSV_BYTES: Long = UPLOAD_CHUNK_BYTES

/** Rows read from Room per page while assembling one upload batch. */
const val FETCH_CHUNK_ROWS: Int = 5000

/** HTTP read/write timeout for large multipart uploads. */
const val UPLOAD_HTTP_TIMEOUT_SEC: Long = 600L

data class UploadCsvBatch(
    val csvText: String,
    val uploadedIds: List<Long>,
    val rowCount: Int,
)

private enum class AppendLineResult {
    Appended,
    CapReached,
}

private fun tryAppendCsvLine(
    body: StringBuilder,
    uploadedIds: MutableList<Long>,
    bodyBytes: Long,
    maxBytes: Long,
    obsId: Long,
    line: String,
): Pair<AppendLineResult, Long> {
    val lineWithNl = "$line\n"
    val lineBytes = lineWithNl.toByteArray(Charsets.UTF_8).size.toLong()
    if (bodyBytes + lineBytes > maxBytes && uploadedIds.isNotEmpty()) {
        return AppendLineResult.CapReached to bodyBytes
    }
    body.append(lineWithNl)
    uploadedIds.add(obsId)
    return AppendLineResult.Appended to (bodyBytes + lineBytes)
}

fun buildUploadBatch(
    observations: List<ObservationEntity>,
    headers: String,
    maxBytes: Long,
    lineFormatter: (ObservationEntity) -> String?,
): UploadCsvBatch {
    val body = StringBuilder(headers)
    val uploadedIds = mutableListOf<Long>()
    var bodyBytes = headers.toByteArray(Charsets.UTF_8).size.toLong()

    for (obs in observations) {
        val line = lineFormatter(obs) ?: continue
        val (result, nextBytes) =
            tryAppendCsvLine(body, uploadedIds, bodyBytes, maxBytes, obs.id, line)
        bodyBytes = nextBytes
        if (result == AppendLineResult.CapReached) {
            break
        }
    }

    return UploadCsvBatch(
        csvText = body.toString(),
        uploadedIds = uploadedIds,
        rowCount = uploadedIds.size,
    )
}

/** Page through one session's pending rows until one byte-capped CSV batch is full. */
suspend fun assembleBatchForSession(
    sessionId: Long,
    headers: String,
    maxBytes: Long,
    fetchChunk: suspend (sessionId: Long, afterId: Long, limit: Int) -> List<ObservationEntity>,
    lineFormatter: (ObservationEntity) -> String?,
): UploadCsvBatch =
    assembleBatchMutex.withLock {
        assembleBatchForSessionUnlocked(sessionId, headers, maxBytes, fetchChunk, lineFormatter)
    }

private suspend fun assembleBatchForSessionUnlocked(
    sessionId: Long,
    headers: String,
    maxBytes: Long,
    fetchChunk: suspend (sessionId: Long, afterId: Long, limit: Int) -> List<ObservationEntity>,
    lineFormatter: (ObservationEntity) -> String?,
): UploadCsvBatch {
    val body = StringBuilder(headers)
    val uploadedIds = mutableListOf<Long>()
    var bodyBytes = headers.toByteArray(Charsets.UTF_8).size.toLong()
    var fetchAfterId = 0L
    var batchFull = false

    while (!batchFull) {
        val chunk = fetchChunk(sessionId, fetchAfterId, FETCH_CHUNK_ROWS)
        if (chunk.isEmpty()) {
            break
        }
        for (obs in chunk) {
            val line = lineFormatter(obs) ?: continue
            val (result, nextBytes) =
                tryAppendCsvLine(body, uploadedIds, bodyBytes, maxBytes, obs.id, line)
            bodyBytes = nextBytes
            if (result == AppendLineResult.CapReached) {
                batchFull = true
                break
            }
        }
        if (batchFull) {
            break
        }
        fetchAfterId = chunk.last().id
        if (chunk.size < FETCH_CHUNK_ROWS) {
            break
        }
    }

    return UploadCsvBatch(
        csvText = body.toString(),
        uploadedIds = uploadedIds,
        rowCount = uploadedIds.size,
    )
}

private val assembleBatchMutex = Mutex()

@Deprecated("Use assembleBatchForSession", ReplaceWith("assembleBatchForSession"))
suspend fun assembleBatchFromMarker(
    marker: Long,
    headers: String,
    maxBytes: Long,
    fetchChunk: suspend (marker: Long, limit: Int) -> List<ObservationEntity>,
    lineFormatter: (ObservationEntity) -> String?,
): UploadCsvBatch =
    assembleBatchMutex.withLock {
        assembleBatchFromMarkerUnlocked(marker, headers, maxBytes, fetchChunk, lineFormatter)
    }

private suspend fun assembleBatchFromMarkerUnlocked(
    marker: Long,
    headers: String,
    maxBytes: Long,
    fetchChunk: suspend (marker: Long, limit: Int) -> List<ObservationEntity>,
    lineFormatter: (ObservationEntity) -> String?,
): UploadCsvBatch {
    val body = StringBuilder(headers)
    val uploadedIds = mutableListOf<Long>()
    var bodyBytes = headers.toByteArray(Charsets.UTF_8).size.toLong()
    var fetchMarker = marker
    var batchFull = false

    while (!batchFull) {
        val chunk = fetchChunk(fetchMarker, FETCH_CHUNK_ROWS)
        if (chunk.isEmpty()) {
            break
        }
        for (obs in chunk) {
            val line = lineFormatter(obs) ?: continue
            val (result, nextBytes) =
                tryAppendCsvLine(body, uploadedIds, bodyBytes, maxBytes, obs.id, line)
            bodyBytes = nextBytes
            if (result == AppendLineResult.CapReached) {
                batchFull = true
                break
            }
        }
        if (batchFull) {
            break
        }
        fetchMarker = chunk.last().id
        if (chunk.size < FETCH_CHUNK_ROWS) {
            break
        }
    }

    return UploadCsvBatch(
        csvText = body.toString(),
        uploadedIds = uploadedIds,
        rowCount = uploadedIds.size,
    )
}

fun observationToJson(obs: ObservationEntity): JSONObject =
    JSONObject().apply {
        put(
            "observation",
            JSONObject().apply {
                put("observed_unix_ms", obs.observedUnixMs)
                put("bssid", obs.bssid)
                put("ssid", obs.ssid)
                put("auth_mode", obs.authMode)
                put("rcois", obs.rcois)
                put("ble_capabilities", obs.bleCapabilities)
                put("channel", obs.channel)
                put("frequency_mhz", obs.frequencyMhz)
                put("rssi", obs.rssi)
                put("lat", obs.lat)
                put("lon", obs.lon)
                obs.altitudeM?.let { put("altitude_m", it) }
                obs.accuracyM?.let { put("accuracy_m", it) }
                put("signal_kind", obs.signalKind)
            },
        )
    }

fun formatUploadCsvLine(obs: ObservationEntity): String? {
    val obsJson = observationToJson(obs)
    return when (obs.signalKind.uppercase()) {
        "BLE" -> NativeCore.formatBleCsvLine(obsJson.toString())
        "WIFI" -> NativeCore.formatWifiCsvLine(obsJson.toString())
        in CellTowerCollector.CELL_SIGNAL_KINDS ->
            NativeCore.formatCellCsvLine(obsJson.toString())
        else -> null
    }
}

@Deprecated("Use formatUploadCsvLine", ReplaceWith("formatUploadCsvLine(obs)"))
fun formatWigleLine(obs: ObservationEntity): String? = formatUploadCsvLine(obs)
