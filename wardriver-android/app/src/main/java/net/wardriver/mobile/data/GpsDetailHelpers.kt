package net.wardriver.mobile.data

import net.wardriver.mobile.ingest.GpsFreshness

/** Dashboard GPS fix detail line (accuracy, altitude, stale age). */
object GpsDetailHelpers {
    const val WAITING_FOR_GPS = "Waiting for GPS"

    /**
     * Idle BALANCED one-shots must not appear as wardrive GPS while a session is active.
     * When [sessionActive], only [wardrive] (possibly null) is shown.
     */
    fun <T> resolveDisplayLocation(
        sessionActive: Boolean,
        wardrive: T?,
        idle: T?,
    ): T? = if (sessionActive) wardrive else idle

    fun formatDetail(
        accuracyM: Float?,
        altitudeM: Double?,
        fixAgeMs: Long?,
        maxFreshAgeMs: Long = GpsFreshness.MAX_FIX_AGE_MS,
    ): String {
        val parts = mutableListOf<String>()
        if (accuracyM != null && accuracyM > 0f) {
            parts.add("%.0f m acc".format(accuracyM))
        }
        if (altitudeM != null) {
            parts.add("%.0f m alt".format(altitudeM))
        }
        val base =
            if (parts.isNotEmpty()) {
                parts.joinToString(" - ")
            } else {
                return "No fix"
            }
        val age = fixAgeMs ?: return base
        return if (age > maxFreshAgeMs) {
            "$base - stale ${age / 1000}s"
        } else {
            base
        }
    }
}
