package net.wardriver.mobile.ui.settings.location

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.LocationSource
import net.wardriver.mobile.ui.components.PackAlertDialog
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveLocalPref
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Plain-language trade shown when the user picks Google fused. */
const val FUSED_LOCATION_DISCLAIMER =
    "GPS only keeps fixes local with no Google involvement. Google fused may use WiFi and cell " +
        "network positioning, which queries Google location service, giving faster fixes indoors " +
        "and before first GPS lock."

/**
 * Session location source: GPS only (default, zero Google) vs opt-in Google fused (faster fix).
 * Picking fused gates on a disclaimer; the mode is persisted only after confirm.
 */
@Composable
fun LocationSourceScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val scope = rememberCoroutineScope()

    val source by appPrefs.locationSourceFlow.collectAsState(initial = LocationSource.GPS_PROVIDER_ONLY)
    var fusedDialogOpen by remember { mutableStateOf(false) }

    SettingsSubScreen(
        title = "Location source",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text(
            if (source == LocationSource.FUSED_OPT_IN) {
                "Google fused (faster fix; uses Google location service)"
            } else {
                "GPS only (default; fixes stay on-device)"
            },
        )

        PackPrimaryButton(
            text = "GPS only (default)",
            onClick = {
                saveLocalPref(scope) { appPrefs.setLocationSource(LocationSource.GPS_PROVIDER_ONLY) }
            },
            modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
        )

        PackOutlinedButton(
            text = "Google fused (faster fix)",
            onClick = { fusedDialogOpen = true },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
    }

    if (fusedDialogOpen) {
        PackAlertDialog(
            onDismissRequest = { fusedDialogOpen = false },
            title = "Use Google fused location?",
            text = {
                Text(FUSED_LOCATION_DISCLAIMER, color = PackColor.Text)
            },
            confirmText = "Use Google fused",
            onConfirm = {
                fusedDialogOpen = false
                scope.launch { appPrefs.setLocationSource(LocationSource.FUSED_OPT_IN) }
            },
            dismissText = "Keep GPS only",
        )
    }
}
