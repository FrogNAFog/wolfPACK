package net.wardriver.mobile.ui.map

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MapAlertDetailSheet(
    pin: AlertPin,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val isDetection = pin.kind.contains("flock", ignoreCase = true) ||
            pin.kind.contains("detection", ignoreCase = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(text = pin.title, style = MaterialTheme.typography.titleMedium)
            when {
                isDetection -> DetectionDetailRows(pin)
                else ->
                    Text(
                        text = pin.detail,
                        style = MaterialTheme.typography.bodyMedium,
                        fontFamily = FontFamily.Monospace,
                        modifier = Modifier.padding(top = 12.dp),
                    )
            }
        }
    }
}

@Composable
private fun DetectionDetailRows(pin: AlertPin) {
    val methodId = pin.detectionMethodId
    Text(
        text =
            if (methodId != null) {
                "Detection ID $methodId"
            } else {
                "Detection ID not recorded for this alert"
            },
        style = MaterialTheme.typography.titleSmall,
        modifier = Modifier.padding(top = 8.dp),
    )
    if (methodId != null) {
        Text(
            text = detectionMethodLabel(methodId),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    Text(
        text = pin.detail,
        style = MaterialTheme.typography.bodyMedium,
        fontFamily = FontFamily.Monospace,
        modifier = Modifier.padding(top = 12.dp),
    )
}

@Composable
internal fun RemoteIdDetailRows(
    pin: AlertPin,
    modifier: Modifier = Modifier,
) {
    val rid = pin.remoteId
    Column(modifier = modifier) {
        DetailRow("Detected", formatRemoteIdDetectionTime(pin.unixMs))
        if (rid == null) {
            Text(
                text = pin.detail,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.padding(top = 12.dp),
            )
            return
        }
        DetailRow("MAC", rid.mac)
        DetailRow("Channel", rid.channel?.toString())
        DetailRow("RSSI", rid.rssi?.let { "$it dBm" })
        DetailRow("Method", remoteIdDetectionMethodLabel(rid.method))
        DetailRow("UAS ID", rid.uasId)
        rid.idType?.let { DetailRow("ID type", remoteIdBasicIdTypeLabel(it)) }
        rid.uaType?.let { DetailRow("UA type", remoteIdUaTypeLabel(it)) }
        rid.status?.let { DetailRow("Status", remoteIdLocationStatusLabel(it)) }
        rid.headingDeg?.let { DetailRow("Heading", "$it deg") }
        rid.speedHMps?.let { DetailRow("Speed", formatOptionalFloat(it, " m/s")) }
        DetailRow("Drone lat", formatOptionalDouble(pin.lat))
        DetailRow("Drone lon", formatOptionalDouble(pin.lon))
        rid.altGeoM?.let { DetailRow("Altitude (geo)", formatOptionalFloat(it, " m")) }
        rid.heightAglM?.let { DetailRow("Height AGL", formatOptionalFloat(it, " m")) }
        rid.heightType?.let { DetailRow("Height type", if (it == 0) "Above takeoff" else "Type $it") }
        rid.horizAccuracy?.let { DetailRow("Horizontal accuracy", remoteIdAccuracyLabel(it)) }
        rid.vertAccuracy?.let { DetailRow("Vertical accuracy", remoteIdAccuracyLabel(it)) }
        rid.speedAccuracy?.let { DetailRow("Speed accuracy", remoteIdAccuracyLabel(it)) }
        rid.baroAccuracy?.let { DetailRow("Baro accuracy", remoteIdAccuracyLabel(it)) }
        rid.tsAccuracy?.let { DetailRow("Timestamp accuracy", remoteIdAccuracyLabel(it)) }
        rid.timestampSec?.let {
            DetailRow("Location timestamp", formatOptionalFloat(it, " s since hour"))
        }
        DetailRow("Operator ID", rid.operatorId)
        rid.operatorIdType?.let { DetailRow("Operator ID type", remoteIdOperatorIdTypeLabel(it)) }
        DetailRow("Operator lat", formatOptionalDouble(rid.operatorLat))
        DetailRow("Operator lon", formatOptionalDouble(rid.operatorLon))
        rid.operatorLocationType?.let {
            DetailRow("Operator location", remoteIdOperatorLocationTypeLabel(it))
        }
    }
}

@Composable
private fun DetailRow(
    label: String,
    value: String?,
) {
    if (value.isNullOrBlank()) return
    Text(
        text = label,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 10.dp),
    )
    Text(
        text = value,
        style = MaterialTheme.typography.bodyMedium,
    )
}
