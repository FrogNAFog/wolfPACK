package net.wardriver.mobile.ingest

import net.wardriver.mobile.NativeCore
import java.util.concurrent.CopyOnWriteArraySet

/**
 * Reassembles concatenated PackLink frames from gateway `SpoolReplay` chunks.
 * Inner frames are emitted serially to an awaited replay ingestion path.
 */
class SpoolReplayAssembler(
    private val onInnerFrame: suspend (ByteArray) -> Unit,
    private val onReplayComplete: suspend (Long) -> Unit = { _ -> },
) {
    /** Awaited listeners invoked for every inner replay frame (e.g. catch-up ingestor). */
    private val replayFrameListeners = CopyOnWriteArraySet<suspend (ByteArray) -> Unit>()

    /** Awaits every registered replay listener for one inner frame (persistence failures propagate). */
    suspend fun notifyReplayFrame(frame: ByteArray) {
        for (listener in replayFrameListeners) {
            listener(frame)
        }
    }

    fun addReplayFrameListener(listener: suspend (ByteArray) -> Unit) {
        replayFrameListeners.add(listener)
    }

    fun removeReplayFrameListener(listener: suspend (ByteArray) -> Unit) {
        replayFrameListeners.remove(listener)
    }

    private var acc = byteArrayOf()
    private var expectedChunkIndex = 0L
    private var expectedOffset = 0L
    private var expectedChunkTotal = 0L
    private var expectedSnapshotNonce = 0L

    suspend fun onSpoolReplayFrame(frame: ByteArray) {
        val chunk = NativeCore.unpackSpoolReplay(frame) ?: return
        val snapshotNonce = packLinkSeq(frame) ?: return
        val payload = hexToBytes(chunk.payloadHex)
        if (chunk.chunkIndex == 0L) {
            acc = byteArrayOf()
            expectedChunkIndex = 0L
            expectedOffset = 0L
            expectedChunkTotal = chunk.chunkTotal
            expectedSnapshotNonce = snapshotNonce
        }
        if (expectedChunkTotal <= 0L ||
            expectedSnapshotNonce == 0L ||
            snapshotNonce != expectedSnapshotNonce ||
            chunk.chunkTotal != expectedChunkTotal ||
            chunk.chunkIndex != expectedChunkIndex ||
            chunk.spoolOffset != expectedOffset
        ) {
            reset()
            return
        }
        acc = acc + payload
        emitCompleteFrames()
        expectedChunkIndex += 1
        expectedOffset += payload.size
        if (expectedChunkIndex == expectedChunkTotal) {
            if (acc.isEmpty()) {
                onReplayComplete(expectedSnapshotNonce)
            }
            resetReplayTracking()
        }
    }

    fun reset() {
        acc = byteArrayOf()
        resetReplayTracking()
    }

    private fun resetReplayTracking() {
        expectedChunkIndex = 0L
        expectedOffset = 0L
        expectedChunkTotal = 0L
        expectedSnapshotNonce = 0L
    }

    private suspend fun emitCompleteFrames() {
        while (true) {
            val (frames, remainder) = extractPackLinkFrames(acc)
            if (frames.isEmpty()) {
                acc = remainder
                return
            }
            for (frame in frames) {
                onInnerFrame(frame)
            }
            acc = remainder
        }
    }

    companion object {
        private const val HEADER_LEN = 14
        private const val CRC_LEN = 4
        private val MAGIC = byteArrayOf(0x50, 0x4C, 0x4E, 0x4B) // PLNK

        fun extractPackLinkFrames(buffer: ByteArray): Pair<List<ByteArray>, ByteArray> {
            val frames = mutableListOf<ByteArray>()
            var offset = 0
            while (offset + HEADER_LEN + CRC_LEN <= buffer.size) {
                if (!buffer.startsWithMagic(offset)) {
                    offset++
                    continue
                }
                val payloadLen =
                    (buffer[offset + 8].toInt() and 0xFF) or
                        ((buffer[offset + 9].toInt() and 0xFF) shl 8)
                val frameLen = HEADER_LEN + payloadLen + CRC_LEN
                if (offset + frameLen > buffer.size) break
                val slice = buffer.copyOfRange(offset, offset + frameLen)
                if (NativeCore.unpackPackLinkFrame(slice) is NativeCore.PackLinkFrameResult.Ok) {
                    frames.add(slice)
                    offset += frameLen
                } else {
                    offset++
                }
            }
            return frames to buffer.copyOfRange(offset, buffer.size)
        }

        private fun ByteArray.startsWithMagic(offset: Int): Boolean =
            offset + 4 <= size &&
                this[offset] == MAGIC[0] &&
                this[offset + 1] == MAGIC[1] &&
                this[offset + 2] == MAGIC[2] &&
                this[offset + 3] == MAGIC[3]

        private fun packLinkSeq(frame: ByteArray): Long? {
            if (frame.size < HEADER_LEN) return null
            return (frame[10].toLong() and 0xFF) or
                ((frame[11].toLong() and 0xFF) shl 8) or
                ((frame[12].toLong() and 0xFF) shl 16) or
                ((frame[13].toLong() and 0xFF) shl 24)
        }

        private fun hexToBytes(hex: String): ByteArray {
            val clean = hex.trim()
            if (clean.length < 2 || clean.length % 2 != 0) return ByteArray(0)
            return ByteArray(clean.length / 2) { i ->
                clean.substring(i * 2, i * 2 + 2).toInt(16).toByte()
            }
        }
    }
}

/** Parsed SpoolReplay chunk from JNI. */
data class SpoolReplayChunk(
    val spoolOffset: Long,
    val chunkTotal: Long,
    val chunkIndex: Long,
    val payloadHex: String,
)
