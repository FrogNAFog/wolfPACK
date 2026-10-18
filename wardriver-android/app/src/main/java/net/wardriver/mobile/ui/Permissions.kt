package net.wardriver.mobile.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Runtime permissions for joining the gateway management SoftAP. */
object WifiPermissions {
    fun canJoinSoftAp(context: Context): Boolean {
        val fine =
            ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.ACCESS_FINE_LOCATION,
            ) == PackageManager.PERMISSION_GRANTED
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val nearby =
                ContextCompat.checkSelfPermission(
                    context,
                    Manifest.permission.NEARBY_WIFI_DEVICES,
                ) == PackageManager.PERMISSION_GRANTED
            return fine && nearby
        }
        return fine
    }
}

/** Bumped when runtime Wi-Fi join permissions change. */
object WifiPermissionNotifier {
    private val _tick = MutableStateFlow(0)
    val tick: StateFlow<Int> = _tick.asStateFlow()

    fun notifyChanged() {
        _tick.value++
    }
}

/**
 * Request runtime permissions needed for gateway SoftAP join, GPS map, and wardrive notifications.
 */
@Composable
fun RequestWardriverPermissions(onPermissionsChanged: (Boolean) -> Unit = {}) {
    val context = LocalContext.current
    val permissions =
        buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                add(Manifest.permission.NEARBY_WIFI_DEVICES)
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
            add(Manifest.permission.ACCESS_FINE_LOCATION)
            add(Manifest.permission.READ_PHONE_STATE)
        }

    val launcher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.RequestMultiplePermissions(),
        ) {
            val granted = WifiPermissions.canJoinSoftAp(context)
            WifiPermissionNotifier.notifyChanged()
            onPermissionsChanged(granted)
        }

    LaunchedEffect(Unit) {
        onPermissionsChanged(WifiPermissions.canJoinSoftAp(context))
        launcher.launch(permissions.toTypedArray())
    }
}
