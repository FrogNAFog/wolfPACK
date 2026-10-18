package net.wardriver.mobile.upload

import org.junit.Assert.assertEquals
import org.junit.Test

class UploadMarkTest {
    @Test
    fun chunkUploadedIds_splitsUnderSqliteLimit() {
        val ids = (1L..2500L).toList()
        val chunks = chunkUploadedIds(ids)
        assertEquals(listOf(900, 900, 700), chunks.map { it.size })
        assertEquals(1L, chunks.first().first())
        assertEquals(2500L, chunks.last().last())
    }
}
