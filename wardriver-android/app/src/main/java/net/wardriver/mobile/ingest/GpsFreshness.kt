package net.wardriver.mobile.ingest

import android.location.Location
import android.os.SystemClock
import net.wardriver.mobile.NativeCore

/** Fused-location freshness for the one-time wardrive cold-start CSV gate. */
object GpsFreshness {
  const val MAX_FIX_AGE_MS = 3_000L

  fun isFresh(loc: Location, nowElapsedNanos: Long = SystemClock.elapsedRealtimeNanos()): Boolean =
      isFreshElapsedNanos(loc.elapsedRealtimeNanos, nowElapsedNanos)

  /** Host-testable freshness check from monotonic fix age. */
  fun isFreshElapsedNanos(fixElapsedNanos: Long, nowElapsedNanos: Long): Boolean {
    if (fixElapsedNanos <= 0L) return false
    val ageMs = (nowElapsedNanos - fixElapsedNanos) / 1_000_000L
    return ageMs in 0..MAX_FIX_AGE_MS
  }
}
