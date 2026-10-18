package net.wardriver.mobile.ui.dashboard

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import net.wardriver.mobile.ui.theme.PackColor

/** Keys for glance metrics that fade green → orange → red when counts stop rising. */
enum class StalenessMetric {
    ObservationRows,
    UniqueWifiMacs,
    UniqueBleDevices,
    CellTowers,
}

/**
 * Tracks last-seen values and the wall time of the last increment per metric.
 * Call [noteSample] only while a wardrive session is active; call [reset] on stop.
 */
class IncrementStalenessTracker {
    private data class Sample(
        val value: Long,
        val lastIncrementAtMs: Long,
    )

    private val samples = mutableMapOf<StalenessMetric, Sample>()

    fun reset() {
        samples.clear()
    }

    /**
     * Records [value] at [nowMs]. If the count rose (or this is the first sample),
     * resets the increment timer so the color returns to green.
     */
    fun noteSample(metric: StalenessMetric, value: Long, nowMs: Long) {
        val prev = samples[metric]
        if (prev == null || value > prev.value) {
            samples[metric] = Sample(value = value, lastIncrementAtMs = nowMs)
        } else {
            // Keep lastIncrementAtMs; still store current value so a later drop+rise works.
            samples[metric] = Sample(value = value, lastIncrementAtMs = prev.lastIncrementAtMs)
        }
    }

    /**
     * Color for [metric] at [nowMs], or null when no sample yet (UI should use [PackColor.Text]).
     */
    fun colorForMetric(metric: StalenessMetric, nowMs: Long): Color? {
        val sample = samples[metric] ?: return null
        return stalenessColor(nowMs - sample.lastIncrementAtMs)
    }

    fun hasSample(metric: StalenessMetric): Boolean = samples.containsKey(metric)

    companion object {
        const val RAMP_MS = 30_000L

        /**
         * Green at 0 ms, orange at mid-ramp, red at [rampMs] and beyond.
         * Midpoint (15 s for default 30 s) is [PackColor.Sun].
         */
        fun stalenessColor(elapsedMs: Long, rampMs: Long = RAMP_MS): Color {
            if (elapsedMs <= 0L) return PackColor.Good
            if (elapsedMs >= rampMs) return PackColor.Err
            val half = rampMs / 2f
            val t = elapsedMs.toFloat()
            return if (t <= half) {
                lerp(PackColor.Good, PackColor.Sun, t / half)
            } else {
                lerp(PackColor.Sun, PackColor.Err, (t - half) / half)
            }
        }
    }
}
