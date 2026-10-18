package net.wardriver.mobile.ingest

import android.content.Context
import android.os.Build
import android.util.Log

/**
 * Wardrive UTC from the Android wall clock when it falls within the APK build window.
 * Does not set the Android OS wall clock.
 */
object WardriveClock {
    private const val TAG = "WardriveClock"

    /** Non-leap 10-year span for the plausibility ceiling. */
    private const val TEN_YEARS_MS = 10L * 365 * 24 * 60 * 60 * 1000

    /** Inclusive floor: [android.os.Build.TIME]. */
    fun floorMs(): Long = Build.TIME

    /** Exclusive ceiling: [floorMs] + 10 years. */
    fun ceilingMs(): Long = floorMs() + TEN_YEARS_MS

    /** Host-testable plausibility check (floor inclusive, ceiling exclusive). */
    fun isPlausible(unixMs: Long, floorMs: Long, ceilingMs: Long): Boolean =
        unixMs in floorMs until ceilingMs

    fun isPlausible(unixMs: Long): Boolean = isPlausible(unixMs, floorMs(), ceilingMs())

    fun isTrusted(): Boolean = isPlausible(System.currentTimeMillis())

    fun nowUnixMs(): Long = System.currentTimeMillis()

    fun register(context: Context) {
        val unixMs = System.currentTimeMillis()
        Log.i(
            TAG,
            "register unixMs=$unixMs floor=${floorMs()} ceiling=${ceilingMs()} trusted=${isTrusted()}",
        )
    }

    fun unregister() {}

    fun reset() {}
}
