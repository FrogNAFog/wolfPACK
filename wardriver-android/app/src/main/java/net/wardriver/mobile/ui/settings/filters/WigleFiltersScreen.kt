package net.wardriver.mobile.ui.settings.filters

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.DenylistBssids
import net.wardriver.mobile.data.LineListValidators
import net.wardriver.mobile.ui.components.SavedMultilineField
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveAndSyncPolicy
import net.wardriver.mobile.wardriverApp

/** BSSID / SSID exclusion lists pushed via policy_sync denylist fields. */
@Composable
fun WigleFiltersScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val scope = rememberCoroutineScope()

    val savedBssidText by appPrefs.denylistBssidsTextFlow.collectAsState(initial = "")
    val savedSsidText by appPrefs.denylistSsidsTextFlow.collectAsState(initial = "")

    SettingsSubScreen(title = "WiGLE filters", onNavigateUp = onNavigateUp, modifier = modifier) {
        Text(
            "One MAC or SSID per line. Gateway SoftAP BSSID is excluded automatically when connected.",
        )
        SavedMultilineField(
            savedValue = savedBssidText,
            label = "BSSID exclusion",
            placeholder = "AA:BB:CC:DD:EE:FF",
            saveButtonText = "Save BSSID list",
            minLines = 5,
            modifier = Modifier.padding(top = 12.dp),
            validate = { text ->
                DenylistBssids.validateMacLines(text)?.let(DenylistBssids::formatValidationMessage)
            },
            normalize = DenylistBssids::normalizeMacLines,
            onSave = { normalized ->
                saveAndSyncPolicy(scope, context) {
                    appPrefs.setDenylistBssidsText(normalized)
                }
            },
        )
        SavedMultilineField(
            savedValue = savedSsidText,
            label = "SSID exclusion",
            saveButtonText = "Save SSID list",
            minLines = 5,
            modifier = Modifier.padding(top = 16.dp),
            validate = LineListValidators::validateSsidLinesMessage,
            normalize = LineListValidators::normalizeSsidLines,
            onSave = { normalized ->
                saveAndSyncPolicy(scope, context) {
                    appPrefs.setDenylistSsidsText(normalized)
                }
            },
        )
    }
}
