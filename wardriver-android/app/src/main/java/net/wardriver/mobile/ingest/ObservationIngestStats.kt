package net.wardriver.mobile.ingest

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.ConcurrentHashMap

/** Session-scoped counters for observation ingest drops (dashboard diagnostics). */
object ObservationIngestStats {
    private val bulksReceived = AtomicLong(0)
    private val bulksEmpty = AtomicLong(0)
    private val bulksUnpackFailed = AtomicLong(0)
    private val bulksIngestFailed = AtomicLong(0)
    private val apsConsidered = AtomicLong(0)
    private val apsStored = AtomicLong(0)
    private val probesStored = AtomicLong(0)
    private val dropReasons = ConcurrentHashMap<String, AtomicLong>()

    private val _snapshot = MutableStateFlow(IngestSnapshot())
    val snapshot: StateFlow<IngestSnapshot> = _snapshot.asStateFlow()

    fun reset() {
        bulksReceived.set(0)
        bulksEmpty.set(0)
        bulksUnpackFailed.set(0)
        bulksIngestFailed.set(0)
        apsConsidered.set(0)
        apsStored.set(0)
        probesStored.set(0)
        dropReasons.clear()
        publish()
    }

    fun recordBulkReceived() {
        bulksReceived.incrementAndGet()
        publish()
    }

    fun recordBulkEmpty() {
        bulksEmpty.incrementAndGet()
        publish()
    }

    fun recordBulkUnpackFailed() {
        bulksUnpackFailed.incrementAndGet()
        publish()
    }

    /** Live bulk ingest threw (Room, JSON, etc.) — caught so wardrive session survives. */
    fun recordBulkIngestFailed(e: Exception) {
        bulksIngestFailed.incrementAndGet()
        val key = "ingest_exception:${e.javaClass.simpleName}"
        dropReasons.computeIfAbsent(key) { AtomicLong(0) }.incrementAndGet()
        publish()
    }

    fun recordApStored() {
        apsStored.incrementAndGet()
        publish()
    }

    fun recordProbeStored(count: Int) {
        probesStored.addAndGet(count.toLong())
        publish()
    }

    fun recordApDropped(reason: String) {
        apsConsidered.incrementAndGet()
        dropReasons.computeIfAbsent(reason) { AtomicLong(0) }.incrementAndGet()
        publish()
    }

    /** Non-fatal ingest setup warning (e.g. enrichment session open failed). */
    fun recordIngestWarning(key: String) {
        dropReasons.computeIfAbsent(key) { AtomicLong(0) }.incrementAndGet()
        publish()
    }

    private fun publish() {
        val drops =
            dropReasons.entries
                .map { (reason, count) -> reason to count.get() }
                .sortedByDescending { it.second }
                .toMap()
        _snapshot.value =
            IngestSnapshot(
                bulksReceived = bulksReceived.get(),
                bulksEmpty = bulksEmpty.get(),
                bulksUnpackFailed = bulksUnpackFailed.get(),
                bulksIngestFailed = bulksIngestFailed.get(),
                apsConsidered = apsConsidered.get(),
                apsStored = apsStored.get(),
                probesStored = probesStored.get(),
                dropReasons = drops,
            )
    }

    data class IngestSnapshot(
        val bulksReceived: Long = 0,
        val bulksEmpty: Long = 0,
        val bulksUnpackFailed: Long = 0,
        val bulksIngestFailed: Long = 0,
        val apsConsidered: Long = 0,
        val apsStored: Long = 0,
        val probesStored: Long = 0,
        val dropReasons: Map<String, Long> = emptyMap(),
    ) {
        /** One-line dashboard foot: top drop reason or bulk receive hint. */
        fun detailLine(packlinkBulks: Long = 0L): String {
            if (bulksReceived == 0L && apsStored == 0L) {
                return if (packlinkBulks == 0L) {
                    "packlink bulks 0"
                } else {
                    "packlink bulks $packlinkBulks - ingest 0"
                }
            }
            val dropped = (apsConsidered - apsStored).coerceAtLeast(0)
            val top =
                dropReasons.entries.maxByOrNull { it.value }?.let { (reason, n) ->
                    "$reason $n"
                }
            return buildString {
                if (packlinkBulks > 0L) append("packlink $packlinkBulks - ")
                append("ingest $bulksReceived - stored $apsStored")
                if (dropped > 0) append(" - dropped $dropped")
                if (bulksEmpty > 0) append(" - empty bulks $bulksEmpty")
                if (bulksUnpackFailed > 0) append(" - unpack fail $bulksUnpackFailed")
                if (bulksIngestFailed > 0) append(" - ingest fail $bulksIngestFailed")
                if (top != null) append(" - top $top")
            }
        }
    }
}
