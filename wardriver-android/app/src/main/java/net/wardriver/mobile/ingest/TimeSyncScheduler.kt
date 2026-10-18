package net.wardriver.mobile.ingest

import android.os.SystemClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import net.wardriver.mobile.NativeCore
import org.json.JSONObject

/**
 * Sends PackLink `TimeSync` uplink on connect, GPS fixes, and periodic heartbeat.
 */
class TimeSyncScheduler(
    private val scope: CoroutineScope,
    private val writeTimeSync: (ByteArray) -> Unit,
) {
    private var timeSetSeq = 0
    private var heartbeatJob: Job? = null

    /** Build wire bytes for a single TimeSync uplink (caller writes on IO thread). */
    @Synchronized
    fun buildWireBytes(): ByteArray? {
        if (!WardriveClock.isTrusted()) return null
        return packWire(gatewayMonoMs = 0)
    }

    /** Explicit post-replay confirmation; the snapshot nonce occupies `time_set_seq`. */
    @Synchronized
    fun buildReplayConfirmationWire(snapshotNonce: Long): ByteArray? {
        if (snapshotNonce <= 0L || snapshotNonce > 0xFFFF_FFFFL) return null
        return packWire(
            gatewayMonoMs = SPOOL_CONFIRM_MARKER,
            timeSetSeqOverride = snapshotNonce.toInt(),
        )
    }

    fun sendNow() {
        val wire = buildWireBytes() ?: return
        writeTimeSync(wire)
    }

    fun startHeartbeat() {
        heartbeatJob?.cancel()
        heartbeatJob =
            scope.launch {
                while (isActive) {
                    delay(HEARTBEAT_MS)
                    sendNow()
                }
            }
    }

    fun stop() {
        heartbeatJob?.cancel()
        heartbeatJob = null
    }

    private fun packWire(
        gatewayMonoMs: Int,
        timeSetSeqOverride: Int? = null,
    ): ByteArray? {
        timeSetSeq = (timeSetSeq + 1) and 0x7FFF_FFFF
        val phoneUnix = WardriveClock.nowUnixMs()
        val req =
            JSONObject()
                .put("phone_unix_ms", phoneUnix)
                .put("phone_mono_ms", SystemClock.elapsedRealtime())
                .put("gateway_mono_ms", gatewayMonoMs.toLong() and 0xFFFF_FFFFL)
                .put("time_set_seq", timeSetSeqOverride ?: timeSetSeq)
                .put("seq", phoneUnix.toInt())
        return NativeCore.packTimeSync(req.toString())
    }

    companion object {
        private const val HEARTBEAT_MS = 30_000L
        private const val SPOOL_CONFIRM_MARKER = 0x5350_4F4C
    }
}
