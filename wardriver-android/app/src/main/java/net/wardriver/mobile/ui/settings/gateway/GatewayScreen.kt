package net.wardriver.mobile.ui.settings.gateway

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackOutlinedTextField
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.components.PackSectionLabel
import net.wardriver.mobile.ui.gateway.GatewayConnectSheet
import net.wardriver.mobile.ui.gateway.GatewayViewModel
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.theme.PackColor

/** Gateway SoftAP link: join, TOFU, credentials, disconnect, forget. */
@Composable
fun GatewayScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: GatewayViewModel =
        viewModel(factory = GatewayViewModel.Factory(LocalContext.current)),
) {
    val connection by viewModel.connectionState.collectAsState()
    val gatewayInfo by viewModel.gatewayInfo.collectAsState()
    val hasTrusted by viewModel.hasTrustedGateway.collectAsState()
    val connectOpen by viewModel.connectSheetOpen.collectAsState()
    val savedSsid by viewModel.softApSsid.collectAsState()
    val savedPass by viewModel.softApPassword.collectAsState()

    var ssid by remember(savedSsid) { mutableStateOf(savedSsid) }
    var password by remember(savedPass) { mutableStateOf(savedPass) }

    val connected = connection is GatewayWifiRepository.ConnectionState.Connected

    SettingsSubScreen(
        title = "Gateway",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text(
            text = "PackLink over gateway SoftAP (192.168.71.1). Cellular stays default for internet.",
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(bottom = 16.dp),
        )

        PackSectionLabel("SoftAP credentials")
        PackOutlinedTextField(
            value = ssid,
            onValueChange = { ssid = it },
            label = { Text("SSID") },
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
            singleLine = true,
        )
        PackOutlinedTextField(
            value = password,
            onValueChange = { password = it },
            label = { Text("Password") },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
            singleLine = true,
        )
        PackOutlinedButton(
            text = if (connected) "Save and push to gateway" else "Save locally",
            onClick = {
                viewModel.saveSoftApCreds(ssid, password, pushToGateway = connected)
            },
            modifier = Modifier.padding(top = 8.dp, bottom = 16.dp),
        )

        if (gatewayInfo != null) {
            PackSectionLabel("Connected gateway")
            Text(
                text = "${gatewayInfo!!.gatewayMac} boot ${gatewayInfo!!.bootId}",
                style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.padding(top = 4.dp, bottom = 8.dp),
            )
        }

        PackSectionLabel("Link status")
        Text(
            text = connectionLabel(connection),
            style = MaterialTheme.typography.bodyLarge,
            color =
                when (connection) {
                    is GatewayWifiRepository.ConnectionState.Connected -> PackColor.Good
                    is GatewayWifiRepository.ConnectionState.Error -> PackColor.Err
                    else -> PackColor.Text
                },
            modifier = Modifier.padding(top = 4.dp, bottom = 16.dp),
        )

        Row(modifier = Modifier.fillMaxWidth()) {
            PackOutlinedButton(
                text = "Connect gateway",
                onClick = viewModel::openConnectSheet,
                modifier = Modifier.padding(end = 8.dp),
            )
            if (!connected && hasTrusted) {
                PackPrimaryButton(
                    text = "Reconnect",
                    onClick = viewModel::connectPreferred,
                )
            }
        }

        if (connected) {
            PackOutlinedButton(
                text = "Disconnect",
                onClick = viewModel::disconnect,
                modifier = Modifier.padding(top = 8.dp),
            )
            PackOutlinedButton(
                text = "Clear fleet binding",
                onClick = viewModel::clearFleetBinding,
                modifier = Modifier.padding(top = 8.dp),
            )
        }

        if (hasTrusted) {
            PackOutlinedButton(
                text = "Forget gateway",
                onClick = viewModel::forgetGateway,
                modifier = Modifier.padding(top = 8.dp),
            )
        }
    }

    if (connectOpen) {
        GatewayConnectSheet(
            viewModel = viewModel,
        )
    }
}

private fun connectionLabel(connection: GatewayWifiRepository.ConnectionState): String =
    when (connection) {
        GatewayWifiRepository.ConnectionState.Connected -> "Connected"
        GatewayWifiRepository.ConnectionState.Connecting -> "Connecting…"
        GatewayWifiRepository.ConnectionState.Reconnecting -> "Reconnecting…"
        is GatewayWifiRepository.ConnectionState.Error -> connection.message
        GatewayWifiRepository.ConnectionState.Disconnected -> "Disconnected"
    }
