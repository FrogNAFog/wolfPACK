package net.wardriver.mobile.upload

import net.wardriver.mobile.db.SessionEntity
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

private val ISO_UPLOAD_FILENAME =
    DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH-mm-ss'Z'").withZone(ZoneOffset.UTC)

/** Filesystem-safe ISO 8601 UTC prefix from session start time (second resolution). */
fun formatSessionDatetimePrefix(startedUnixMs: Long): String =
    ISO_UPLOAD_FILENAME.format(Instant.ofEpochMilli(startedUnixMs))

/**
 * WiGLE multipart filename for one session chunk.
 * Chunk 1 has no numeric suffix; chunks 2+ use `_02`, `_03`, …
 * When [useSessionIdInfix], inserts `_s{sessionId}` before the chunk suffix.
 */
fun uploadCsvFileName(
    session: SessionEntity,
    chunkIndex: Int,
    useSessionIdInfix: Boolean,
): String {
    val base = formatSessionDatetimePrefix(session.startedUnixMs)
    val infix = if (useSessionIdInfix) "_s${session.id}" else ""
    val chunkSuffix =
        when {
            chunkIndex <= 1 -> ""
            chunkIndex < 10 -> "_0$chunkIndex"
            else -> "_$chunkIndex"
        }
    return "$base$infix$chunkSuffix.csv"
}
