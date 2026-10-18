package net.wardriver.mobile.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

/** Settings hub: links to all preference sub-screens. */
@Composable
fun SettingsHubScreen(
    onNavigateUploads: () -> Unit,
    onNavigateFilters: () -> Unit,
    onNavigateDetections: () -> Unit,
    onNavigateHomeZone: () -> Unit,
    onNavigateLocation: () -> Unit,
    onNavigateNotifications: () -> Unit,
    onNavigateNetwork: () -> Unit,
    onNavigateCapture: () -> Unit,
    onNavigateGateway: () -> Unit,
    onNavigateChannelPlan: () -> Unit,
    onNavigateMap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxSize().padding(vertical = 8.dp)) {
        Text(
            text = "Settings",
            style = MaterialTheme.typography.headlineSmall,
            color = PackColor.Cyan,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        HorizontalDivider(color = PackColor.BorderChromeSubtle)

        SettingsHubItem(
            title = "Uploads",
            subtitle = "WiGLE and wdgwars credentials",
            onClick = onNavigateUploads,
        )
        SettingsHubItem(
            title = "WiGLE filters",
            subtitle = "BSSID and SSID exclusion lists",
            onClick = onNavigateFilters,
        )
        SettingsHubItem(
            title = "Detections",
            subtitle = "Detection methods, MAC ignore, SSID watch",
            onClick = onNavigateDetections,
        )
        SettingsHubItem(
            title = "Home zone",
            subtitle = "Suppress CSV inside home radius",
            onClick = onNavigateHomeZone,
        )
        SettingsHubItem(
            title = "Location source",
            subtitle = "GPS only (default) or Google fused",
            onClick = onNavigateLocation,
        )
        SettingsHubItem(
            title = "Notifications",
            subtitle = "Detection delivery mode, sound, and vibration",
            onClick = onNavigateNotifications,
        )
        SettingsHubItem(
            title = "Network",
            subtitle = "Connectivity and cellular upload policy",
            onClick = onNavigateNetwork,
        )
        SettingsHubItem(
            title = "Gateway",
            subtitle = "Adopt, connect, and forget wolfPACK gateway",
            onClick = onNavigateGateway,
        )
        SettingsHubItem(
            title = "Channel plan",
            subtitle = "Dwell, band pools, and hop schedule",
            onClick = onNavigateChannelPlan,
        )
        SettingsHubItem(
            title = "Map",
            subtitle = "Basemap style (light or dark tiles)",
            onClick = onNavigateMap,
        )
        SettingsHubItem(
            title = "Capture",
            subtitle = "60 s WiFi probe capture via gateway",
            onClick = onNavigateCapture,
        )
    }
}

@Composable
private fun SettingsHubItem(
    title: String,
    subtitle: String,
    onClick: () -> Unit,
) {
    ListItem(
        headlineContent = { Text(title, color = PackColor.Text) },
        supportingContent = { Text(subtitle, color = PackColor.Muted) },
        colors =
            ListItemDefaults.colors(
                containerColor = PackColor.Background,
            ),
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick),
    )
    HorizontalDivider(color = PackColor.BorderChromeSubtle)
}
