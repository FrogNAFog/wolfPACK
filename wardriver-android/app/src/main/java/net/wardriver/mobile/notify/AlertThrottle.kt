package net.wardriver.mobile.notify

import java.util.concurrent.ConcurrentHashMap

/** Result of per-MAC notify cooldown check. */
sealed class NotifyDecision {
    data class Post(val suppressedCount: Int) : NotifyDecision()
    data object Suppress : NotifyDecision()
}

/**
 * In-memory per-MAC detection alert throttle (default 30 s).
 * Uses stable notification IDs so repeated alerts update one shade row.
 */
object AlertThrottle {
    private val macRegex = Regex("""mac=([0-9a-fA-F:]+)""")
    private val jsonMacRegex = Regex(""""mac"\s*:\s*"([^"]+)"""", RegexOption.IGNORE_CASE)

    private data class ThrottleState(
        var lastNotifiedMs: Long,
        var suppressedCount: Int,
    )

    private val perKey = ConcurrentHashMap<String, ThrottleState>()

    fun parseMac(detail: String): String? {
        val trimmed = detail.trimStart()
        if (trimmed.startsWith("{")) {
            try {
                val json = org.json.JSONObject(detail)
                val mac = json.optString("mac", "").trim()
                if (mac.isNotEmpty()) {
                    return mac.lowercase()
                }
            } catch (_: Exception) {
                // fall through to legacy `mac=` token
            }
        }
        jsonMacRegex.find(detail)?.groupValues?.get(1)?.lowercase()?.let { return it }
        return macRegex.find(detail)?.groupValues?.get(1)?.lowercase()
    }

    fun throttleKey(kind: String, mac: String?): String =
        if (mac != null) {
            "$kind:$mac"
        } else {
            "$kind:unknown"
        }

    /**
     * Returns [NotifyDecision.Post] when a user-visible alert may fire (includes prior
     * suppressed count for messaging). Returns [NotifyDecision.Suppress] during cooldown.
     */
    fun shouldNotify(
        key: String,
        nowMs: Long,
        cooldownMs: Long,
        skipThrottle: Boolean = false,
    ): NotifyDecision {
        if (skipThrottle) return NotifyDecision.Post(0)

        val state = perKey.getOrPut(key) { ThrottleState(lastNotifiedMs = 0, suppressedCount = 0) }
        if (state.lastNotifiedMs > 0 && nowMs - state.lastNotifiedMs < cooldownMs) {
            state.suppressedCount++
            evictStale(nowMs, cooldownMs * 2)
            return NotifyDecision.Suppress
        }
        val suppressed = state.suppressedCount
        state.lastNotifiedMs = nowMs
        state.suppressedCount = 0
        evictStale(nowMs, cooldownMs * 2)
        return NotifyDecision.Post(suppressed)
    }

    fun stableNotificationId(mac: String): Int =
        AlertChannels.DETECTION_NOTIFICATION_BASE_ID + (mac.hashCode() and 0x7FFF)

    fun fallbackNotificationId(kind: String, unixMs: Long): Int =
        AlertChannels.DETECTION_NOTIFICATION_BASE_ID + 8000 +
            ((kind.hashCode() xor unixMs.toInt()) and 0x7FF)

    private fun evictStale(nowMs: Long, maxAgeMs: Long) {
        if (perKey.size < 256) return
        val cutoff = nowMs - maxAgeMs
        perKey.entries.removeIf { (_, state) ->
            state.lastNotifiedMs > 0 && state.lastNotifiedMs < cutoff
        }
    }
}
