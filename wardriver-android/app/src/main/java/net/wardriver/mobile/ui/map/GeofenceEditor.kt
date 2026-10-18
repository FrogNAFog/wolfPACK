package net.wardriver.mobile.ui.map

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.theme.PackColor
import org.osmdroid.util.GeoPoint

/**
 * Geofence square editor controls. Map tap handling lives in [MapScreen];
 * this composable toggles edit mode and applies/clears bounds.
 */
@Composable
fun GeofenceEditor(
    geofence: GeofenceBounds?,
    coverageFraction: Double,
    editMode: Boolean,
    onEditModeChange: (Boolean) -> Unit,
    onClear: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier
            .fillMaxWidth()
            .background(PackColor.Surface)
            .padding(horizontal = 8.dp, vertical = 8.dp),
    ) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            PackPrimaryButton(
                text = if (editMode) "Done drawing" else "Edit geofence",
                onClick = { onEditModeChange(!editMode) },
                modifier = Modifier.weight(1f),
            )
            if (geofence != null) {
                PackOutlinedButton(
                    text = "Clear",
                    onClick = onClear,
                )
            }
        }
        val hint = when {
            editMode -> "Tap two opposite corners on the map to set the square."
            geofence != null -> {
                val pct = (coverageFraction * 100).toInt()
                "Geofence active - coverage $pct% inside square."
            }
            else -> "No geofence. Tap Edit geofence to draw a square."
        }
        Text(
            hint,
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

/** Tracks two-corner rectangle selection while geofence edit mode is active. */
class GeofenceDrawState {
    var cornerA: GeoPoint? by mutableStateOf(null)
    var cornerB: GeoPoint? by mutableStateOf(null)

    fun reset() {
        cornerA = null
        cornerB = null
    }

    fun onMapTap(point: GeoPoint, onComplete: (GeofenceBounds) -> Unit) {
        if (cornerA == null) {
            cornerA = point
            cornerB = null
            return
        }
        cornerB = point
        val a = cornerA!!
        val bounds = GeofenceBounds(
            latMin = minOf(a.latitude, point.latitude),
            latMax = maxOf(a.latitude, point.latitude),
            lonMin = minOf(a.longitude, point.longitude),
            lonMax = maxOf(a.longitude, point.longitude),
        )
        onComplete(bounds)
        reset()
    }

    fun previewBounds(): GeofenceBounds? {
        val a = cornerA ?: return null
        val b = cornerB ?: return null
        return GeofenceBounds(
            latMin = minOf(a.latitude, b.latitude),
            latMax = maxOf(a.latitude, b.latitude),
            lonMin = minOf(a.longitude, b.longitude),
            lonMax = maxOf(a.longitude, b.longitude),
        )
    }
}

@Composable
fun rememberGeofenceDrawState(): GeofenceDrawState = remember { GeofenceDrawState() }
