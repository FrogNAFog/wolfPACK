package net.wardriver.mobile.ui.map

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MapDeviceDetailSheet(
    cluster: ApCluster,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val sortedMembers = cluster.members.sortedByDescending { it.rssi }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
        ) {
            Text(
                text =
                    if (cluster.count > 1) {
                        "${cluster.count} devices at this location"
                    } else {
                        cluster.labelSsid
                    },
                style = MaterialTheme.typography.titleMedium,
            )
            LazyColumn(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 12.dp, bottom = 24.dp),
            ) {
                items(sortedMembers, key = { it.bssid }) { member ->
                    Column(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                        Text(text = member.ssid.ifBlank { "(hidden)" }, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            text = member.bssid,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            text = "${member.signalKind} - RSSI ${member.rssi} dBm",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                    HorizontalDivider()
                }
            }
        }
    }
}
