package net.wardriver.mobile.ui.settings.homezone

import android.Manifest
import android.content.pm.PackageManager
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.OneShotLocation
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackOutlinedTextField
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveAndSyncPolicy
import net.wardriver.mobile.wardriverApp

/** Home zone circle: suppresses enrichment CSV when phone GPS is inside. */
@Composable
fun HomeZoneScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val appPrefs = context.wardriverApp().appPreferences
    val scope = rememberCoroutineScope()

    val enabled by appPrefs.homeZoneEnabledFlow.collectAsState(initial = false)
    val lat by appPrefs.homeZoneLatFlow.collectAsState(initial = null)
    val lon by appPrefs.homeZoneLonFlow.collectAsState(initial = null)
    val radius by appPrefs.homeZoneRadiusMFlow.collectAsState(
        initial = AppPreferences.DEFAULT_HOME_ZONE_RADIUS_M,
    )

    var latField by remember(lat) { mutableStateOf(lat?.toString().orEmpty()) }
    var lonField by remember(lon) { mutableStateOf(lon?.toString().orEmpty()) }
    var radiusField by remember(radius) { mutableStateOf(radius.toString()) }
    var status by remember { mutableStateOf<String?>(null) }

    SettingsSubScreen(
        title = "Home zone",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text(
            if (enabled) "Home zone active - CSV enqueue blocked inside radius"
            else "Home zone disabled",
        )
        PackOutlinedTextField(
            value = latField,
            onValueChange = { latField = it },
            label = { Text("Latitude") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
            singleLine = true,
        )
        PackOutlinedTextField(
            value = lonField,
            onValueChange = { lonField = it },
            label = { Text("Longitude") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
            singleLine = true,
        )
        PackOutlinedTextField(
            value = radiusField,
            onValueChange = { radiusField = it },
            label = { Text("Radius (m)") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
            singleLine = true,
        )

        PackPrimaryButton(
            text = "Save home zone",
            onClick = {
                val parsedLat = latField.toDoubleOrNull()
                val parsedLon = lonField.toDoubleOrNull()
                val parsedRadius = radiusField.toDoubleOrNull()
                if (parsedLat == null || parsedLon == null || parsedRadius == null || parsedRadius <= 0) {
                    status = "Enter valid lat, lon, and radius"
                    return@PackPrimaryButton
                }
                saveAndSyncPolicy(scope, context) {
                    appPrefs.saveHomeZone(parsedLat, parsedLon, parsedRadius)
                }
                status = "Saved"
            },
            modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
        )

        PackPrimaryButton(
            text = "Use current GPS",
            onClick = {
                scope.launch {
                    val loc = fetchCurrentLocation(context)
                    if (loc == null) {
                        status = "GPS unavailable or permission denied"
                        return@launch
                    }
                    latField = loc.first.toString()
                    lonField = loc.second.toString()
                    val parsedRadius = radiusField.toDoubleOrNull() ?: AppPreferences.DEFAULT_HOME_ZONE_RADIUS_M
                    saveAndSyncPolicy(scope, context) {
                        appPrefs.saveHomeZone(loc.first, loc.second, parsedRadius)
                    }
                    status = "Set from current GPS"
                }
            },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )

        PackOutlinedButton(
            text = "Clear home zone",
            onClick = {
                saveAndSyncPolicy(scope, context) { appPrefs.clearHomeZone() }
                latField = ""
                lonField = ""
                status = "Cleared"
            },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )

        status?.let { Text(it, modifier = Modifier.padding(top = 8.dp)) }
    }
}

private suspend fun fetchCurrentLocation(context: android.content.Context): Pair<Double, Double>? {
    val fine =
        ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED
    if (!fine) return null
    val loc = OneShotLocation.resolve(context) ?: return null
    return loc.latitude to loc.longitude
}
