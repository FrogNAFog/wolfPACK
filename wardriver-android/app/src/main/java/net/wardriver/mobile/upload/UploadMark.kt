package net.wardriver.mobile.upload

import net.wardriver.mobile.db.ObservationDao

/** Stay under SQLite's 999 bind-variable limit per `IN` clause (post-POST marks only). */
const val MARK_UPLOADED_CHUNK_SIZE = 900

/** Split observation ids into upload-mark chunks (host-testable). */
fun chunkUploadedIds(
    ids: List<Long>,
    chunkSize: Int = MARK_UPLOADED_CHUNK_SIZE,
): List<List<Long>> = ids.chunked(chunkSize)

/** Mark many rows without exceeding SQLite bind limits (after HTTP POST succeeds). */
suspend fun ObservationDao.markUploadedIdsChunked(
    ids: List<Long>,
    dest: UploadDest,
) {
    for (chunk in chunkUploadedIds(ids)) {
        when (dest) {
            UploadDest.WIGLE -> markWigleUploadedIds(chunk)
            UploadDest.WDGWARS -> markWdgwarsUploadedIds(chunk)
        }
    }
}
