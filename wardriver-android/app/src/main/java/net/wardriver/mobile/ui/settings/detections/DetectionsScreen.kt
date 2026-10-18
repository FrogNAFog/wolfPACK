package net.wardriver.mobile.ui.settings.detections

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.foundation.layout.Row
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.DenylistBssids
import net.wardriver.mobile.data.LineListValidators
import net.wardriver.mobile.ui.components.PackSwitch
import net.wardriver.mobile.ui.components.SavedMultilineField
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveAndSyncPolicy
import net.wardriver.mobile.ui.settings.saveLocalPref
import net.wardriver.mobile.wardriverApp

/** Detection method toggles, MAC ignore list, and SSID watch (policy_sync tail). */
@Composable
fun DetectionsScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val scope = rememberCoroutineScope()

    val wifiMask by appPrefs.detectionDisableWifiMaskFlow.collectAsState(
        initial = AppPreferences.DEFAULT_DETECTION_DISABLE_WIFI_MASK,
    )
    val bleMask by appPrefs.detectionDisableBleMaskFlow.collectAsState(
        initial = AppPreferences.DEFAULT_DETECTION_DISABLE_BLE_MASK,
    )
    val bleActiveScan by appPrefs.bleActiveScanFlow.collectAsState(initial = false)
    val savedMacIgnore by appPrefs.detectionMacIgnoreTextFlow.collectAsState(initial = "")
    val savedSsidWatch by appPrefs.ssidWatchTextFlow.collectAsState(initial = "")

    SettingsSubScreen(title = "Detections", onNavigateUp = onNavigateUp, modifier = modifier) {
        Text("WiFi detection methods (checked = enabled)")
        DetectionCheckbox(
            label = "1 - Wildcard probe + Flock OUI (deprecated)",
            checked = AppPreferences.isWifiMethodEnabled(wifiMask, 1),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableWifiMaskFlow.first()
                    appPrefs.setDetectionDisableWifiMask(
                        AppPreferences.setWifiMethodEnabled(current, 1, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "2 - Wildcard probe + IE signature + Flock OUI",
            checked = AppPreferences.isWifiMethodEnabled(wifiMask, 2),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableWifiMaskFlow.first()
                    appPrefs.setDetectionDisableWifiMask(
                        AppPreferences.setWifiMethodEnabled(current, 2, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "3 - Wildcard probe + IE signature (any MAC)",
            checked = AppPreferences.isWifiMethodEnabled(wifiMask, 3),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableWifiMaskFlow.first()
                    appPrefs.setDetectionDisableWifiMask(
                        AppPreferences.setWifiMethodEnabled(current, 3, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "4 - Axon OUI (mobile surveillance Wi-Fi)",
            checked = AppPreferences.isWifiMethodEnabled(wifiMask, 4),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableWifiMaskFlow.first()
                    appPrefs.setDetectionDisableWifiMask(
                        AppPreferences.setWifiMethodEnabled(current, 4, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "5 - i-PRO OUI (probe-request Wi-Fi)",
            checked = AppPreferences.isWifiMethodEnabled(wifiMask, 5),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableWifiMaskFlow.first()
                    appPrefs.setDetectionDisableWifiMask(
                        AppPreferences.setWifiMethodEnabled(current, 5, enabled),
                    )
                }
            },
        )

        Text("BLE methods (checked = enabled)", modifier = Modifier.padding(top = 16.dp))
        DetectionCheckbox(
            label = "32 - Xuntong manufacturer ID (deprecated)",
            checked = AppPreferences.isBleMethodEnabled(bleMask, 32),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableBleMaskFlow.first()
                    appPrefs.setDetectionDisableBleMask(
                        AppPreferences.setBleMethodEnabled(current, 32, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "33 - Advertised name keyword (deprecated)",
            checked = AppPreferences.isBleMethodEnabled(bleMask, 33),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableBleMaskFlow.first()
                    appPrefs.setDetectionDisableBleMask(
                        AppPreferences.setBleMethodEnabled(current, 33, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "34 - MAC OUI infrastructure (deprecated)",
            checked = AppPreferences.isBleMethodEnabled(bleMask, 34),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableBleMaskFlow.first()
                    appPrefs.setDetectionDisableBleMask(
                        AppPreferences.setBleMethodEnabled(current, 34, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "35 - Axon OUI (mobile surveillance)",
            checked = AppPreferences.isBleMethodEnabled(bleMask, 35),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableBleMaskFlow.first()
                    appPrefs.setDetectionDisableBleMask(
                        AppPreferences.setBleMethodEnabled(current, 35, enabled),
                    )
                }
            },
        )
        DetectionCheckbox(
            label = "36 - i-PRO OUI (mobile surveillance)",
            checked = AppPreferences.isBleMethodEnabled(bleMask, 36),
            onCheckedChange = { enabled ->
                saveAndSyncPolicy(scope, context) {
                    val current = appPrefs.detectionDisableBleMaskFlow.first()
                    appPrefs.setDetectionDisableBleMask(
                        AppPreferences.setBleMethodEnabled(current, 36, enabled),
                    )
                }
            },
        )

        Text("BLE scan mode", modifier = Modifier.padding(top = 16.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            PackSwitch(
                checked = bleActiveScan,
                onCheckedChange = { enabled ->
                    saveAndSyncPolicy(scope, context) { appPrefs.setBleActiveScan(enabled) }
                },
            )
            Text(
                text = "Active scan (SCAN_REQ)",
                modifier = Modifier.padding(start = 4.dp),
            )
        }
        Text(
            text = "Send SCAN_REQ so devices may return scan-response AD (name / manufacturer data). Default off = passive.",
            modifier = Modifier.padding(top = 4.dp),
        )

        Text("Detection MAC ignore (phone-local)", modifier = Modifier.padding(top = 16.dp))
        SavedMultilineField(
            savedValue = savedMacIgnore,
            label = "MACs to ignore",
            placeholder = "AA:BB:CC:DD:EE:FF",
            saveButtonText = "Save MAC ignore list",
            minLines = 4,
            modifier = Modifier.padding(top = 8.dp),
            validate = { text ->
                DenylistBssids.validateMacLines(text)?.let(DenylistBssids::formatValidationMessage)
            },
            normalize = DenylistBssids::normalizeMacLines,
            onSave = { normalized ->
                saveLocalPref(scope) { appPrefs.setDetectionMacIgnoreText(normalized) }
            },
        )

        Text("SSID watch (synced to gateway)", modifier = Modifier.padding(top = 16.dp))
        Text(
            text = "Exact SSIDs, plus PREFIX:MLG, PREFIX_LEN:MLG,8, CONTAINS:AXON",
            modifier = Modifier.padding(top = 4.dp),
        )
        SavedMultilineField(
            savedValue = savedSsidWatch,
            label = "SSID watch list",
            saveButtonText = "Save SSID watch list",
            minLines = 4,
            modifier = Modifier.padding(top = 8.dp),
            validate = LineListValidators::validateSsidLinesMessage,
            normalize = LineListValidators::normalizeSsidLines,
            onSave = { normalized ->
                saveAndSyncPolicy(scope, context) {
                    appPrefs.setSsidWatchText(normalized)
                }
            },
        )
    }
}

@Composable
private fun DetectionCheckbox(
    label: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = onCheckedChange)
        Text(text = label, modifier = Modifier.padding(start = 4.dp))
    }
}
