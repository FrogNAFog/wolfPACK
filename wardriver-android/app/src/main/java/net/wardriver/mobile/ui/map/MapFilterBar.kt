package net.wardriver.mobile.ui.map

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun MapFilterBar(
    layerFilters: MapLayerFilters,
    geofenceActive: Boolean,
    coverageShadingEnabled: Boolean,
    onLayerFiltersChange: (MapLayerFilters) -> Unit,
    onCoverageShadingChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val chipColors =
        FilterChipDefaults.filterChipColors(
            selectedContainerColor = PackColor.Surface2,
            selectedLabelColor = PackColor.Cyan,
            containerColor = PackColor.Surface,
            labelColor = PackColor.Muted,
        )
    val panelModifier =
        modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 4.dp)
            .clip(RoundedCornerShape(bottomStart = 8.dp, bottomEnd = 8.dp))
            .background(PackColor.Surface)
            .border(1.dp, PackColor.BorderCyan, RoundedCornerShape(bottomStart = 8.dp, bottomEnd = 8.dp))
            .padding(horizontal = 8.dp, vertical = 8.dp)

    Column(panelModifier) {
        Text(
            "Map layers",
            style = MaterialTheme.typography.labelMedium,
            color = PackColor.Muted,
        )
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(top = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            FilterChip(
                selected = layerFilters.wifi,
                onClick = { onLayerFiltersChange(layerFilters.copy(wifi = !layerFilters.wifi)) },
                label = { Text("WiFi") },
                colors = chipColors,
            )
            FilterChip(
                selected = layerFilters.ble,
                onClick = { onLayerFiltersChange(layerFilters.copy(ble = !layerFilters.ble)) },
                label = { Text("BLE") },
                colors = chipColors,
            )
            FilterChip(
                selected = layerFilters.probe,
                onClick = { onLayerFiltersChange(layerFilters.copy(probe = !layerFilters.probe)) },
                label = { Text("Probe") },
                colors = chipColors,
            )
            FilterChip(
                selected = layerFilters.alerts,
                onClick = { onLayerFiltersChange(layerFilters.copy(alerts = !layerFilters.alerts)) },
                label = { Text("Alerts") },
                colors = chipColors,
            )
            if (geofenceActive) {
                FilterChip(
                    selected = coverageShadingEnabled,
                    onClick = { onCoverageShadingChange(!coverageShadingEnabled) },
                    label = { Text("Show coverage") },
                    colors = chipColors,
                )
            }
        }
    }
}
