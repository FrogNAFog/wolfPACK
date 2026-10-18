package net.wardriver.mobile.ui.dashboard

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.location.LocationManager
import android.os.Build
import androidx.core.content.ContextCompat
import net.wardriver.mobile.data.LocationSource
import net.wardriver.mobile.ingest.WardriveClock
import net.wardriver.mobile.wardriverApp

/** Preflight checks before starting wardrive foreground service. */
object WardriveStartPreflight {
    const val PHONE_TIME_UNSET_MESSAGE = "Phone time unset - set date and time"

    /** One-line marker shown when the user has opted into Google fused location. */
    const val FUSED_OPT_IN_MARKER =
        "Google fused location active (opt-in; queries Google location service)"

    data class Result(
        val canStart: Boolean,
        val warnings: List<String> = emptyList(),
        val hardBlockReason: String? = null,
    )

    /** Host-testable clock gate (mirrors [evaluate] clock check). */
    fun clockBlocksStart(clockOk: Boolean): String? =
        if (clockOk) null else PHONE_TIME_UNSET_MESSAGE

    suspend fun evaluate(context: Context): Result {
        val warnings = mutableListOf<String>()
        if (!WardriveClock.isTrusted()) {
            warnings.add(PHONE_TIME_UNSET_MESSAGE)
            return Result(
                canStart = false,
                warnings = warnings,
                hardBlockReason = PHONE_TIME_UNSET_MESSAGE,
            )
        }
        val fine =
            ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) ==
                PackageManager.PERMISSION_GRANTED
        if (!fine) {
            warnings.add("Location permission missing - logging stays closed until granted")
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val notifications =
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
                    PackageManager.PERMISSION_GRANTED
            if (!notifications) {
                warnings.add("Notifications denied - wardrive may not start in background")
            }
        }
        if (fine) {
            val lm = context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
            val gpsEnabled =
                lm.isProviderEnabled(LocationManager.GPS_PROVIDER) ||
                    lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER)
            if (!gpsEnabled) {
                warnings.add("GPS off - enable location for wardrive logging")
            }
        }
        val locationSource = context.wardriverApp().appPreferences.locationSource()
        if (locationSource == LocationSource.FUSED_OPT_IN) {
            warnings.add(FUSED_OPT_IN_MARKER)
        }
        return Result(canStart = true, warnings = warnings)
    }
}
