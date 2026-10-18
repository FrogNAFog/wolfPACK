package net.wardriver.mobile.ui.map

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

/**
 * Modal bottom sheet for map layer filters, data scope, and text search.
 * Replaces the always-visible [MapFilterBar].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MapLayerSheet(
    layerFilters: MapLayerFilters,
    searchQuery: String,
    searchResultCount: Int,
    geofenceActive: Boolean,
    coverageShadingEnabled: Boolean,
    onLayerFiltersChange: (MapLayerFilters) -> Unit,
    onSearchQueryChange: (String) -> Unit,
    onCoverageShadingChange: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    var internalQuery by remember { mutableStateOf(searchQuery) }
    LaunchedEffect(searchQuery) { internalQuery = searchQuery }

    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = PackColor.Surface,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
        ) {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
            ) {
                Text(
                    "Map layers",
                    style = MaterialTheme.typography.titleMedium,
                    color = PackColor.Text,
                )
                TextButton(onClick = onDismiss) {
                    Text("Close", color = PackColor.Text)
                }
            }

            // Search field
            OutlinedTextField(
                value = internalQuery,
                onValueChange = { newVal ->
                    internalQuery = newVal
                    onSearchQueryChange(newVal)
                },
                placeholder = { Text("Search BSSID, SSID, or BLE name...", color = PackColor.Muted) },
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 4.dp),
                leadingIcon = {
                    Icon(Icons.Default.Search, contentDescription = "Search", tint = PackColor.Muted)
                },
                trailingIcon = {
                    if (internalQuery.isNotEmpty())
                        IconButton(onClick = {
                            internalQuery = ""
                            onSearchQueryChange("")
                        }) {
                            Icon(Icons.Default.Clear, contentDescription = "Clear", tint = PackColor.Muted)
                        }
                },
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyMedium.copy(color = PackColor.Text),
                colors = androidx.compose.material3.OutlinedTextFieldDefaults.colors(
                    focusedContainerColor = PackColor.Surface2,
                    unfocusedContainerColor = PackColor.Surface2,
                    focusedBorderColor = PackColor.Cyan,
                    unfocusedBorderColor = PackColor.BorderCyan,
                ),
            )

            // Result count
            if (internalQuery.isNotEmpty()) {
                Text(
                    "$searchResultCount results",
                    style = MaterialTheme.typography.labelSmall,
                    color = PackColor.Sun,
                    modifier = Modifier.padding(start = 4.dp, top = 2.dp),
                )
            }

            // Layer filter chips
            val chipColors =
                FilterChipDefaults.filterChipColors(
                    selectedContainerColor = PackColor.Surface2,
                    selectedLabelColor = PackColor.Cyan,
                    containerColor = PackColor.Surface2.copy(alpha = 0.5f),
                    labelColor = PackColor.Muted,
                )

            Column(Modifier.padding(top = 8.dp)) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 6.dp),
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
                }

                // --- Alerts: master toggle + expandable submenu ---
                var showAlertSubmenu by remember { mutableStateOf(layerFilters.alerts) }
                LaunchedEffect(layerFilters.alerts) {
                    if (!layerFilters.alerts) showAlertSubmenu = false
                }

                // Master Alerts chip row
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 6.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    FilterChip(
                        selected = layerFilters.alerts,
                        onClick = {
                            val newAlerts = !layerFilters.alerts
                            if (newAlerts) {
                                // Turning master ON: enable all per-type toggles that were OFF
                                val updated = layerFilters.copy(
                                    alerts = true,
                                    alertDetectionWifi = true,
                                    alertDetectionBle = true,
                                    alertSsidWatch = true,
                                    alertRemoteId = true,
                                    alertAxon = true,
                                    alertIPro = true,
                                )
                                onLayerFiltersChange(updated)
                                showAlertSubmenu = true
                            } else {
                                // Turning master OFF: disable all per-type toggles
                                val updated = layerFilters.copy(
                                    alerts = false,
                                    alertDetectionWifi = false,
                                    alertDetectionBle = false,
                                    alertSsidWatch = false,
                                    alertRemoteId = false,
                                    alertAxon = false,
                                    alertIPro = false,
                                )
                                onLayerFiltersChange(updated)
                                showAlertSubmenu = false
                            }
                        },
                        label = {
                            if (layerFilters.alerts && layerFilters.alertEnabledCount < 6) {
                                Text("Alerts (${layerFilters.alertEnabledCount})")
                            } else {
                                Text("Alerts")
                            }
                        },
                        trailingIcon = {
                            Icon(
                                Icons.Filled.ArrowDropDown,
                                contentDescription = "Expand alert types",
                                tint = PackColor.Muted,
                                modifier = Modifier.padding(end = 2.dp),
                            )
                        },
                        colors = chipColors,
                    )
                    // Expand/collapse toggle
                    if (layerFilters.alerts) {
                        IconButton(onClick = { showAlertSubmenu = !showAlertSubmenu }) {
                            Icon(
                                Icons.Filled.ArrowDropDown,
                                contentDescription = if (showAlertSubmenu) "Collapse" else "Expand",
                                tint = PackColor.Cyan,
                                modifier = if (showAlertSubmenu) Modifier.rotate(180f) else Modifier,
                            )
                        }
                    }
                }

                // Expandable alert type submenu
                AnimatedVisibility(
                    visible = layerFilters.alerts && showAlertSubmenu,
                    enter = expandVertically() + fadeIn(),
                    exit = shrinkVertically() + fadeOut(),
                ) {
                    Column(Modifier.padding(start = 8.dp, top = 4.dp)) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            FilterChip(
                                selected = layerFilters.alertDetectionWifi,
                                onClick = { onLayerFiltersChange(layerFilters.copy(alertDetectionWifi = !layerFilters.alertDetectionWifi)) },
                                label = { Text("Camera WiFi") },
                                colors = chipColors,
                            )
                            FilterChip(
                                selected = layerFilters.alertDetectionBle,
                                onClick = { onLayerFiltersChange(layerFilters.copy(alertDetectionBle = !layerFilters.alertDetectionBle)) },
                                label = { Text("Detection BLE") },
                                colors = chipColors,
                            )
                        }
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(top = 6.dp),
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            FilterChip(
                                selected = layerFilters.alertSsidWatch,
                                onClick = { onLayerFiltersChange(layerFilters.copy(alertSsidWatch = !layerFilters.alertSsidWatch)) },
                                label = { Text("SSID Watch") },
                                colors = chipColors,
                            )
                            FilterChip(
                                selected = layerFilters.alertRemoteId,
                                onClick = { onLayerFiltersChange(layerFilters.copy(alertRemoteId = !layerFilters.alertRemoteId)) },
                                label = { Text("Remote ID") },
                                colors = chipColors,
                            )
                        }
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(top = 6.dp),
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            FilterChip(
                                selected = layerFilters.alertAxon,
                                onClick = { onLayerFiltersChange(layerFilters.copy(alertAxon = !layerFilters.alertAxon)) },
                                label = { Text("Axon") },
                                colors = chipColors,
                            )
                            FilterChip(
                                selected = layerFilters.alertIPro,
                                onClick = { onLayerFiltersChange(layerFilters.copy(alertIPro = !layerFilters.alertIPro)) },
                                label = { Text("i-PRO") },
                                colors = chipColors,
                            )
                        }
                    }
                }

                // Coverage shading (if geofence active)
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 6.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    if (geofenceActive) {
                        FilterChip(
                            selected = coverageShadingEnabled,
                            onClick = { onCoverageShadingChange(!coverageShadingEnabled) },
                            label = { Text("Coverage") },
                            colors = chipColors,
                        )
                    }
                }
            }

            Spacer(Modifier.height(32.dp))
        }
    }
}
