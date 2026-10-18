package net.wardriver.mobile.ui.settings.map

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
import net.wardriver.mobile.ui.components.PackRadioButton
import net.wardriver.mobile.ui.map.MapBasemap
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveLocalPref
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Map display preferences (basemap style). */
@Composable
fun MapSettingsScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val basemap by appPrefs.mapBasemapFlow.collectAsState(initial = MapBasemap.Light)
    val scope = rememberCoroutineScope()

    SettingsSubScreen(
        title = "Map",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text(
            text = "Basemap",
            style = MaterialTheme.typography.titleMedium,
            color = PackColor.Cyan,
        )
        Text(
            text =
                "Light uses OpenStreetMap (default). Dark uses CARTO dark tiles. " +
                    "Switching clears cached tiles for the previous style.",
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
            modifier = Modifier.padding(top = 4.dp, bottom = 8.dp),
        )
        Column(Modifier.selectableGroup()) {
            BasemapOption(
                label = "Light",
                selected = basemap == MapBasemap.Light,
                onSelect = {
                    saveLocalPref(scope) { appPrefs.setMapBasemap(MapBasemap.Light) }
                },
            )
            BasemapOption(
                label = "Dark",
                selected = basemap == MapBasemap.Dark,
                onSelect = {
                    saveLocalPref(scope) { appPrefs.setMapBasemap(MapBasemap.Dark) }
                },
            )
        }
    }
}

@Composable
private fun BasemapOption(
    label: String,
    selected: Boolean,
    onSelect: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .selectable(
                    selected = selected,
                    onClick = onSelect,
                    role = Role.RadioButton,
                )
                .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PackRadioButton(selected = selected, onClick = onSelect)
        Text(
            text = label,
            color = PackColor.Text,
            modifier = Modifier.padding(start = 8.dp),
        )
    }
}
