package net.wardriver.mobile.ui.nav

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Dashboard
import androidx.compose.material.icons.filled.Map
import androidx.compose.material.icons.filled.Radar
import androidx.compose.material.icons.filled.Settings
import androidx.compose.ui.graphics.vector.ImageVector

/** Bottom navigation destinations for the fleet control shell. */
enum class AppDestinations(
    val route: String,
    val label: String,
    val icon: ImageVector,
) {
    Dashboard("dashboard", "Dashboard", Icons.Filled.Dashboard),
    Sniffers("sniffers", "Sniffers", Icons.Filled.Radar),
    Map("map", "Map", Icons.Filled.Map),
    Settings("settings", "Settings", Icons.Filled.Settings),
}
