package net.wardriver.mobile.ui.map

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.db.SessionWithCount
import net.wardriver.mobile.ui.theme.PackColor
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val SESSION_DATE_FORMAT = SimpleDateFormat("MMM d, yyyy h:mm a", Locale.getDefault())

/**
 * Modal bottom sheet for browsing and selecting historical wardrive sessions.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionPickerSheet(
    sessions: List<SessionWithCount>,
    hasActiveWardrive: Boolean,
    selectedSessionId: Long?,
    dataScope: MapDataScope,
    onSelectSession: (Long?) -> Unit,
    onSelectAllDevices: () -> Unit,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = PackColor.Surface,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        ) {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "Select session",
                    style = MaterialTheme.typography.titleMedium,
                    color = PackColor.Text,
                )
                TextButton(onClick = onDismiss) {
                    Text("Close", color = PackColor.Text)
                }
            }

            // "All devices" option
            SessionRow(
                label = "All devices",
                detail = "Show all observations across all sessions",
                count = null,
                isSelected = dataScope == MapDataScope.AllDevices,
                onClick = onSelectAllDevices,
                modifier = Modifier.padding(top = 4.dp),
            )

            // "This session (active)" option
            if (hasActiveWardrive) {
                SessionRow(
                    label = "This session (active)",
                    detail = "Currently recording",
                    count = null,
                    isSelected = selectedSessionId == null && dataScope == MapDataScope.Session,
                    onClick = { onSelectSession(null) },
                    modifier = Modifier.padding(top = 4.dp),
                    highlight = true,
                )
            }

            LazyColumn(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .padding(top = 4.dp, bottom = 16.dp),
            ) {
                items(sessions, key = { it.session.id }) { swc ->
                    SessionRow(
                        label = formatSessionDate(swc.session.startedUnixMs),
                        detail = "Gateway: ${swc.session.gatewayMac.takeLast(8)}",
                        count = swc.observationCount,
                        isSelected = selectedSessionId == swc.session.id,
                        onClick = { onSelectSession(swc.session.id) },
                    )
                }
            }
        }
    }
}

@Composable
private fun SessionRow(
    label: String,
    detail: String,
    count: Int?,
    isSelected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    highlight: Boolean = false,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = 12.dp, horizontal = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                text = label,
                style = MaterialTheme.typography.bodyMedium,
                color = if (highlight) PackColor.Cyan else PackColor.Text,
                fontWeight = if (highlight || isSelected) FontWeight.Bold else FontWeight.Normal,
            )
            Text(
                text = detail,
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Muted,
            )
        }
        count?.let { c ->
            Text(
                text = "$c obs",
                style = MaterialTheme.typography.labelSmall,
                color = if (isSelected) PackColor.Sun else PackColor.Muted,
                modifier = Modifier.padding(start = 8.dp),
            )
        }
    }
}

private fun formatSessionDate(unixMs: Long): String {
    return try {
        SESSION_DATE_FORMAT.format(Date(unixMs))
    } catch (_: Exception) {
        unixMs.toString()
    }
}
