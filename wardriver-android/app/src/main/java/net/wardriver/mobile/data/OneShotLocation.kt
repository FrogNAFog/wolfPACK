package net.wardriver.mobile.data

import android.content.Context
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Handler
import android.os.Looper
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import com.google.android.gms.tasks.CancellationTokenSource
import kotlinx.coroutines.suspendCancellableCoroutine
import net.wardriver.mobile.wardriverApp
import kotlin.coroutines.resume

/**
 * Resolves a single location fix while respecting the persisted [LocationSource]
 * (DataStore key `location_source`).
 *
 * [LocationSource.FUSED_OPT_IN] (user opt-in) keeps the existing Google fused
 * one-shot call — the Play Services path the app used before this gate existed.
 *
 * [LocationSource.GPS_PROVIDER_ONLY] (the privacy default) makes zero Play
 * Services location calls: it reads the platform [LocationManager] last-known
 * GPS fix and, only when there is none, arms a short one-shot platform GPS
 * update before settling on the first fix (or timing out to null).
 *
 * Used by the dashboard glance, the map recenter, and the home-zone "use current
 * GPS" action so a one-shot fix never silently hits Google when the user chose
 * GPS-only.
 */
object OneShotLocation {

    /** Max wait for a fresh platform GPS fix before settling (no fused fallback). */
    private const val FRESH_FIX_TIMEOUT_MS = 8_000L
    private const val FRESH_FIX_MIN_INTERVAL_MS = 0L
    private const val FRESH_FIX_MIN_DISTANCE_M = 0f

    /**
     * One-shot fix honoring the persisted location source.
     *
     * @return the best available [Location] (fresh fix preferred, last-known
     *   fallback), or null when no location is available / permission is missing.
     */
    suspend fun resolve(context: Context): Location? {
        val source = context.wardriverApp().appPreferences.locationSource()
        return if (source == LocationSource.FUSED_OPT_IN) {
            fusedOneShot(context)
        } else {
            gpsOnlyOneShot(context)
        }
    }

    /** Google fused one-shot (opt-in): fresh fix with last-known fallback. */
    private suspend fun fusedOneShot(context: Context): Location? {
        return suspendCancellableCoroutine { cont ->
            val fused = LocationServices.getFusedLocationProviderClient(context)
            val token = CancellationTokenSource()
            fused.getCurrentLocation(Priority.PRIORITY_BALANCED_POWER_ACCURACY, token.token)
                .addOnSuccessListener { loc ->
                    if (cont.isActive) {
                        if (loc != null) {
                            cont.resume(loc)
                        } else {
                            fused.lastLocation.addOnSuccessListener { cont.resume(it) }
                        }
                    }
                }
                .addOnFailureListener {
                    if (cont.isActive) {
                        fused.lastLocation.addOnSuccessListener { cont.resume(it) }
                    }
                }
        }
    }

    /**
     * Platform GPS-only one-shot: last-known GPS fix first; if absent, a short
     * one-shot [LocationManager.requestLocationUpdates] for a fresh fix.
     */
    private suspend fun gpsOnlyOneShot(context: Context): Location? {
        return try {
            val lm = context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
            val lastKnown = lm.getLastKnownLocation(LocationManager.GPS_PROVIDER)
            if (lastKnown != null) {
                lastKnown
            } else {
                armFreshGpsFix(lm)
            }
        } catch (_: SecurityException) {
            // Permission flow handles grant; leave the fix stale until granted.
            null
        }
    }

    /**
     * Arm a short one-shot platform GPS update, settle on the first fix, and clean
     * up (remove updates + timeout) once resumed, timed out, or cancelled.
     */
    private suspend fun armFreshGpsFix(lm: LocationManager): Location? {
        return suspendCancellableCoroutine { cont ->
            val handler = Handler(Looper.getMainLooper())
            lateinit var listener: LocationListener
            lateinit var timeout: Runnable
            fun cleanup() {
                try {
                    lm.removeUpdates(listener)
                } catch (_: SecurityException) {
                }
                handler.removeCallbacks(timeout)
            }
            listener =
                LocationListener { loc ->
                    if (cont.isActive) {
                        cleanup()
                        cont.resume(loc)
                    }
                }
            timeout =
                Runnable {
                    if (cont.isActive) {
                        cleanup()
                        cont.resume(null)
                    }
                }
            try {
                lm.requestLocationUpdates(
                    LocationManager.GPS_PROVIDER,
                    FRESH_FIX_MIN_INTERVAL_MS,
                    FRESH_FIX_MIN_DISTANCE_M,
                    listener,
                    Looper.getMainLooper(),
                )
            } catch (_: SecurityException) {
                if (cont.isActive) cont.resume(null)
                return@suspendCancellableCoroutine
            }
            handler.postDelayed(timeout, FRESH_FIX_TIMEOUT_MS)
            cont.invokeOnCancellation {
                cleanup()
            }
        }
    }
}
