package net.wardriver.mobile.ui.settings.network

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import net.wardriver.mobile.ui.components.PackSwitch
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveAndSyncPolicy
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Connectivity readout and upload-on-cellular policy toggle. */
@Composable
fun NetworkScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val scope = rememberCoroutineScope()

    val uploadOnCellular by appPrefs.uploadOnCellularFlow.collectAsState(initial = false)
    val cellTowersEnabled by appPrefs.cellTowersEnabledFlow.collectAsState(initial = true)
    val connectivity = describeConnectivity(context)

    SettingsSubScreen(
        title = "Network",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text("Current link: $connectivity")
        Text(
            "Upload workers respect the cellular toggle below.",
            modifier = Modifier.padding(top = 8.dp, bottom = 16.dp),
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(text = "Upload on cellular", modifier = Modifier.weight(1f), color = PackColor.Text)
            PackSwitch(
                checked = uploadOnCellular,
                onCheckedChange = { v ->
                    saveAndSyncPolicy(scope, context) { appPrefs.setUploadOnCellular(v) }
                },
            )
        }
        Text(
            "Cell towers are logged from this phone only; gateway sniffers are unchanged.",
            modifier = Modifier.padding(top = 16.dp, bottom = 8.dp),
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(text = "Log cell towers", modifier = Modifier.weight(1f), color = PackColor.Text)
            PackSwitch(
                checked = cellTowersEnabled,
                onCheckedChange = { v -> scope.launch { appPrefs.setCellTowersEnabled(v) } },
            )
        }
    }
}

private fun describeConnectivity(context: Context): String {
    val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        ?: return "unknown"
    val network = cm.activeNetwork ?: return "offline"
    val caps = cm.getNetworkCapabilities(network) ?: return "offline"
    return when {
        caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "WiFi"
        caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
        caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> "ethernet"
        else -> "other"
    }
}
