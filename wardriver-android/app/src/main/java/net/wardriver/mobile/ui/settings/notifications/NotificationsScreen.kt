package net.wardriver.mobile.ui.settings.notifications

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AlertDeliveryMode
import net.wardriver.mobile.ui.components.PackRadioButton
import net.wardriver.mobile.ui.components.PackSwitch
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Detection alert delivery mode, sound, and vibration toggles. */
@Composable
fun NotificationsScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val scope = rememberCoroutineScope()

    val detectionAlerts by appPrefs.detectionAlertsEnabledFlow.collectAsState(initial = true)
    val deliveryMode by appPrefs.detectionDeliveryModeFlow.collectAsState(initial = AlertDeliveryMode.InAppWhenForeground)
    val sound by appPrefs.soundDetectionFlow.collectAsState(initial = true)
    val vibrate by appPrefs.vibrateDetectionFlow.collectAsState(initial = true)

    val systemSoundApplies = detectionAlerts && deliveryMode == AlertDeliveryMode.Full

    SettingsSubScreen(
        title = "Notifications",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text(
            text = "Detection alerts are throttled to once every 30 seconds per MAC.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(bottom = 8.dp),
        )
        NotificationToggle(
            label = "Detection alerts",
            checked = detectionAlerts,
            onCheckedChange = { enabled ->
                scope.launch { appPrefs.setDetectionAlertsEnabled(enabled) }
            },
        )
        Text(
            text = "Delivery",
            style = MaterialTheme.typography.labelMedium,
            modifier = Modifier.padding(top = 12.dp, bottom = 4.dp),
        )
        Column(Modifier.selectableGroup()) {
            DeliveryModeOption(
                label = "In-app when open",
                description = "Top banner while app is visible; notification when background",
                selected = deliveryMode == AlertDeliveryMode.InAppWhenForeground,
                enabled = detectionAlerts,
                onSelect = { scope.launch { appPrefs.setDetectionDeliveryMode(AlertDeliveryMode.InAppWhenForeground) } },
            )
            DeliveryModeOption(
                label = "Full alert",
                description = "Heads-up notification with optional sound and vibration",
                selected = deliveryMode == AlertDeliveryMode.Full,
                enabled = detectionAlerts,
                onSelect = { scope.launch { appPrefs.setDetectionDeliveryMode(AlertDeliveryMode.Full) } },
            )
            DeliveryModeOption(
                label = "Quiet",
                description = "Shade only, no heads-up popup",
                selected = deliveryMode == AlertDeliveryMode.Quiet,
                enabled = detectionAlerts,
                onSelect = { scope.launch { appPrefs.setDetectionDeliveryMode(AlertDeliveryMode.Quiet) } },
            )
        }
        NotificationToggle(
            label = "Sound",
            checked = sound,
            enabled = systemSoundApplies,
            onCheckedChange = { enabled ->
                scope.launch { appPrefs.setSoundDetection(enabled) }
            },
            modifier = Modifier.padding(top = 8.dp),
        )
        NotificationToggle(
            label = "Vibration",
            checked = vibrate,
            enabled = systemSoundApplies,
            onCheckedChange = { enabled ->
                scope.launch { appPrefs.setVibrateDetection(enabled) }
            },
        )
        if (deliveryMode == AlertDeliveryMode.InAppWhenForeground) {
            Text(
                text = "Sound and vibration apply to background notifications only.",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        Text(
            text = "Geofence",
            style = MaterialTheme.typography.labelMedium,
            modifier = Modifier.padding(top = 16.dp, bottom = 4.dp),
        )
        Text(
            text = "Alerts when GPS enters or leaves the wardrive square on the map.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(bottom = 8.dp),
        )
        val geofenceAlerts by appPrefs.geofenceAlertsEnabledFlow.collectAsState(initial = true)
        NotificationToggle(
            label = "Geofence alerts",
            checked = geofenceAlerts,
            onCheckedChange = { enabled ->
                scope.launch { appPrefs.setGeofenceAlertsEnabled(enabled) }
            },
        )
        Text(
            text = "Uses sound and vibration toggles above when the app is in the background.",
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

@Composable
private fun DeliveryModeOption(
    label: String,
    description: String,
    selected: Boolean,
    enabled: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .selectable(
                    selected = selected,
                    enabled = enabled,
                    role = Role.RadioButton,
                    onClick = onSelect,
                )
                .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PackRadioButton(
            selected = selected,
            onClick = if (enabled) onSelect else null,
            enabled = enabled,
        )
        Column(Modifier.padding(start = 8.dp)) {
            Text(
                text = label,
                color =
                    if (enabled) {
                        PackColor.Text
                    } else {
                        PackColor.Muted.copy(alpha = 0.38f)
                    },
            )
            Text(
                text = description,
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Muted,
            )
        }
    }
}

@Composable
private fun NotificationToggle(
    label: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            modifier = Modifier.weight(1f),
            color =
                if (enabled) {
                    PackColor.Text
                } else {
                    PackColor.Muted.copy(alpha = 0.38f)
                },
        )
        PackSwitch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            enabled = enabled,
        )
    }
}
