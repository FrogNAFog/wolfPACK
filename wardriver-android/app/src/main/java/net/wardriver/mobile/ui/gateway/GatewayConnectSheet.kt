package net.wardriver.mobile.ui.gateway

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.theme.PackColor

/**
 * Modal sheet to join the gateway SoftAP and complete TOFU on first connect.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GatewayConnectSheet(
    viewModel: GatewayViewModel,
    modifier: Modifier = Modifier,
) {
    val connectState by viewModel.connectState.collectAsState()
    val connection by viewModel.connectionState.collectAsState()
    val pendingTofu by viewModel.pendingTofu.collectAsState()
    val gatewayInfo by viewModel.gatewayInfo.collectAsState()
    val canConnect = viewModel.canConnect()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    LaunchedEffect(connection) {
        if (connection is GatewayWifiRepository.ConnectionState.Connected) {
            viewModel.closeConnectSheet()
        }
    }

    LaunchedEffect(canConnect, pendingTofu) {
        if (canConnect && pendingTofu == null && connection is GatewayWifiRepository.ConnectionState.Disconnected) {
            viewModel.connect()
        }
    }

    ModalBottomSheet(
        onDismissRequest = viewModel::dismissConnectSheet,
        sheetState = sheetState,
        modifier = modifier,
        containerColor = PackColor.Surface,
        contentColor = PackColor.Text,
        dragHandle = {
            androidx.compose.material3.BottomSheetDefaults.DragHandle(
                color = PackColor.Muted,
            )
        },
    ) {
        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp).padding(bottom = 24.dp)) {
            Text(
                text = "Connect gateway",
                style = MaterialTheme.typography.titleLarge,
                color = PackColor.Cyan,
            )
            Text(
                text =
                    "Joins the wolfPACK gateway Wi-Fi (cellular stays on for maps). " +
                        "First phone to connect is pinned to this gateway.",
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Muted,
                modifier = Modifier.padding(top = 8.dp, bottom = 12.dp),
            )

            if (!canConnect) {
                Text(
                    text = "Grant location and Nearby Wi-Fi devices permissions to join the gateway AP.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = PackColor.Err,
                )
                return@Column
            }

            if (pendingTofu != null) {
                TofuConfirmPanel(
                    gatewayInfo = gatewayInfo,
                    onConfirm = viewModel::confirmPendingTofu,
                    onCancel = viewModel::cancelPendingTofu,
                )
                return@Column
            }

            when (val state = connectState) {
                is GatewayWifiRepository.ConnectState.Failed -> {
                    Text(
                        text = state.message,
                        style = MaterialTheme.typography.bodyMedium,
                        color = PackColor.Err,
                    )
                    PackOutlinedButton(
                        text = "Retry",
                        onClick = viewModel::connect,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                }
                GatewayWifiRepository.ConnectState.Joining -> LoadingRow("Joining gateway Wi-Fi…")
                else -> {
                    if (connection is GatewayWifiRepository.ConnectionState.Connecting) {
                        LoadingRow("Connecting PackLink…")
                    }
                }
            }
        }
    }
}

@Composable
private fun TofuConfirmPanel(
    gatewayInfo: GatewayWifiRepository.GatewayInfo?,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
) {
    Column {
        Text(
            text = "Trust this gateway?",
            style = MaterialTheme.typography.titleMedium,
            color = PackColor.Cyan,
        )
        if (gatewayInfo != null) {
            Text(
                text = "MAC: ${gatewayInfo.gatewayMac}",
                style = MaterialTheme.typography.bodyMedium,
                color = PackColor.Text,
                modifier = Modifier.padding(top = 8.dp),
            )
            Text(
                text = "Boot ID: ${gatewayInfo.bootId}",
                style = MaterialTheme.typography.bodyMedium,
                color = PackColor.Text,
            )
        }
        PackPrimaryButton(
            text = "Pin this phone",
            onClick = onConfirm,
            modifier = Modifier.padding(top = 16.dp),
        )
        PackOutlinedButton(
            text = "Cancel",
            onClick = onCancel,
            modifier = Modifier.padding(top = 8.dp),
        )
    }
}

@Composable
private fun LoadingRow(label: String) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator(color = PackColor.Cyan)
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = PackColor.Muted,
            modifier = Modifier.padding(top = 12.dp),
        )
    }
}
