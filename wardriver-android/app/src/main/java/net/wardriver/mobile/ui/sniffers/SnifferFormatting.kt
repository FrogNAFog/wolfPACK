package net.wardriver.mobile.ui.sniffers

import net.wardriver.mobile.NativeCore
import org.json.JSONObject

/** Gateway-relative stale threshold (matches control web `SNIFFER_QUIET_STALE_MS`). */
const val SNIFFER_QUIET_STALE_MS = 20_000L

private fun snifferDisplayWithGatewayFields(
    op: String,
    gatewayNowMs: Long,
    lastSeenMs: Long,
    gatewayConnected: Boolean,
): String? =
    try {
        val req =
            JSONObject()
                .put("gateway_now_ms", gatewayNowMs)
                .put("last_seen_ms", lastSeenMs)
                .put("gateway_connected", gatewayConnected)
        NativeCore.snifferDisplay(op, req)
    } catch (_: RuntimeException) {
        // JVM unit tests stub android.jar JSONObject; use Kotlin fallbacks below.
        null
    }

/** Control-style stale: `gatewayNowMs - lastSeenMs >= 20s`. */
fun snifferIsQuietStale(
    gatewayNowMs: Long,
    lastSeenMs: Long,
    gatewayConnected: Boolean,
): Boolean =
    snifferDisplayWithGatewayFields("quiet_stale", gatewayNowMs, lastSeenMs, gatewayConnected)
        ?.toBooleanStrictOrNull()
        ?: kotlinQuietStale(gatewayNowMs, lastSeenMs, gatewayConnected)

/**
 * Unadopted Nearby row visibility: connected and heard within the last 20 seconds.
 * Returns false when disconnected or the gateway clock is invalid.
 */
fun pendingNearbyVisible(
    gatewayNowMs: Long,
    lastSeenMs: Long,
    gatewayConnected: Boolean,
): Boolean =
    snifferDisplayWithGatewayFields("pending_visible", gatewayNowMs, lastSeenMs, gatewayConnected)
        ?.toBooleanStrictOrNull()
        ?: (!gatewayConnected || gatewayNowMs <= 0L || lastSeenMs <= 0L).not() &&
        gatewayNowMs - lastSeenMs < SNIFFER_QUIET_STALE_MS

/** True when a Nearby row is shown but last heard exceeds the 20-second liveness window. */
fun pendingNearbyStale(
    gatewayNowMs: Long,
    lastSeenMs: Long,
    gatewayConnected: Boolean,
): Boolean =
    snifferDisplayWithGatewayFields("pending_stale", gatewayNowMs, lastSeenMs, gatewayConnected)
        ?.toBooleanStrictOrNull()
        ?: (pendingNearbyVisible(gatewayNowMs, lastSeenMs, gatewayConnected).not() &&
            gatewayConnected &&
            gatewayNowMs > 0L &&
            lastSeenMs > 0L)

/** Control-style relative last-seen (second precision under 1 minute). */
fun formatSnifferLastSeenAgo(gatewayNowMs: Long, lastSeenMs: Long?): String {
    val native =
        try {
            val req =
                JSONObject()
                    .put("gateway_now_ms", gatewayNowMs)
                    .put("last_seen_ms", lastSeenMs ?: 0L)
            NativeCore.snifferDisplay("last_seen_ago", req)
        } catch (_: RuntimeException) {
            null
        }
    return native ?: kotlinLastSeenAgo(gatewayNowMs, lastSeenMs)
}

/** Unadopted Nearby rows: not rostered; fresh rows first, stale rows kept visible. */
fun filterPendingNearby(
    nearby: List<net.wardriver.mobile.data.FleetRepository.SnifferStatusRow>,
    adoptedIds: Set<Int>,
    pendingIds: Set<Int>,
    gatewayNowMs: Long,
    gatewayConnected: Boolean,
): List<net.wardriver.mobile.data.FleetRepository.SnifferStatusRow> =
    nearby
        .filter { it.snifferId !in adoptedIds && it.snifferId !in pendingIds }
        .sortedWith(
            compareByDescending<net.wardriver.mobile.data.FleetRepository.SnifferStatusRow> {
                pendingNearbyVisible(gatewayNowMs, it.lastSeenMs, gatewayConnected)
            }.thenByDescending { it.lastSeenMs },
        )

/** Show em dash when gateway status not received yet. */
fun formatPct(value: Int?): String = value?.toString() ?: "-"

/** wolfPACK `SnifferState` wire label. */
fun stateLabel(state: Int?): String {
    val native =
        try {
            val req = JSONObject().put("state", state ?: JSONObject.NULL)
            NativeCore.snifferDisplay("state_label", req)
        } catch (_: RuntimeException) {
            null
        }
    return native ?: kotlinStateLabel(state)
}

/** Short display ID (last 4 hex digits of u32). */
fun shortSnifferId(snifferId: Int): String {
    val native =
        try {
            val req = JSONObject().put("sniffer_id", snifferId)
            NativeCore.snifferDisplay("short_id", req)
        } catch (_: RuntimeException) {
            null
        }
    return native ?: kotlinShortSnifferId(snifferId)
}

private fun kotlinQuietStale(gatewayNowMs: Long, lastSeenMs: Long, gatewayConnected: Boolean): Boolean {
    if (!gatewayConnected) return false
    if (gatewayNowMs <= 0L || lastSeenMs <= 0L) return false
    return gatewayNowMs - lastSeenMs >= SNIFFER_QUIET_STALE_MS
}

private fun kotlinLastSeenAgo(gatewayNowMs: Long, lastSeenMs: Long?): String {
    if (lastSeenMs == null || lastSeenMs <= 0L) return "-"
    val d = gatewayNowMs - lastSeenMs
    if (d < 0 || d > 86_400_000L) return "-"
    if (d < 1500) return "just now"
    val sec = d / 1000
    if (sec < 60) return "${sec}s ago"
    return "${sec / 60}m ago"
}

private fun kotlinStateLabel(state: Int?): String =
    when (state) {
        1 -> "SCANNING"
        0 -> "IDLE"
        null -> "-"
        else -> "ERROR"
    }

private fun kotlinShortSnifferId(snifferId: Int): String {
    val hex = (snifferId.toLong() and 0xFFFF_FFFF).toString(16).uppercase().padStart(8, '0')
    return "...${hex.takeLast(4)}"
}
