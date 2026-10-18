package net.wardriver.mobile.ui.sniffers

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.ui.components.PackSnackbarHost
import net.wardriver.mobile.ui.theme.PackColor

/** Fleet roster management (Sniffers tab). */
@Composable
fun SniffersScreen(
    modifier: Modifier = Modifier,
    onNavigateGatewaySettings: (() -> Unit)? = null,
    viewModel: SniffersViewModel = viewModel(factory = SniffersViewModel.Factory(LocalContext.current)),
) {
    val uiState by viewModel.uiState.collectAsState()
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(Unit) {
        viewModel.ensureGatewayConnected()
    }

    LaunchedEffect(uiState.snackbarMessage) {
        val message = uiState.snackbarMessage ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        viewModel.clearSnackbar()
    }

    val gatewayConnected = uiState.connection is GatewayWifiRepository.ConnectionState.Connected
    val gatewayNowMs = uiState.gatewayNowMs
    val liveStatusById = uiState.nearbySniffers.associateBy { it.snifferId }

    Scaffold(
        modifier = modifier,
        snackbarHost = { PackSnackbarHost(snackbarHostState) },
    ) { innerPadding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(innerPadding)
                    .padding(16.dp),
        ) {
            Text(
                text = "Sniffers",
                style = MaterialTheme.typography.headlineSmall,
                color = PackColor.Cyan,
            )
            Text(
                text = connectionLabel(uiState.connection),
                style = MaterialTheme.typography.bodyMedium,
                color = PackColor.Muted,
                modifier = Modifier.padding(top = 4.dp, bottom = 12.dp),
            )

            NearbySniffersSection(
                pending = uiState.pendingNearby,
                connection = uiState.connection,
                gatewayNowMs = gatewayNowMs,
                onAdopt = viewModel::adopt,
                onRetryConnect = viewModel::ensureGatewayConnected,
                onOpenGatewaySettings = onNavigateGatewaySettings,
                modifier = Modifier.fillMaxWidth(),
            )

            if (uiState.adoptPending.isNotEmpty()) {
                HorizontalDivider(
                    modifier = Modifier.padding(vertical = 12.dp),
                    color = PackColor.BorderChromeSubtle,
                )
                AdoptingSniffersSection(
                    pending = uiState.adoptPending,
                    liveStatusById = liveStatusById,
                    gatewayNowMs = gatewayNowMs,
                    gatewayConnected = gatewayConnected,
                    policyAckOk = uiState.lastPolicyAckOk,
                    adoptFailedIds = uiState.adoptFailedIds,
                    onCancel = viewModel::cancelAdopt,
                    onRetry = viewModel::retryAdopt,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            HorizontalDivider(
                modifier = Modifier.padding(vertical = 12.dp),
                color = PackColor.BorderChromeSubtle,
            )

            FleetRosterSection(
                roster = uiState.adoptedRoster,
                liveStatusById = liveStatusById,
                gatewayNowMs = gatewayNowMs,
                gatewayConnected = gatewayConnected,
                manualBandMode = uiState.channelPlan.isManualBandMode(),
                onRemove = viewModel::remove,
                onSetRole = viewModel::setRole,
                onSetBand = viewModel::setBand,
                onSetPinnedChannel = viewModel::setPinnedChannel,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

private fun connectionLabel(connection: GatewayWifiRepository.ConnectionState): String =
    when (connection) {
        GatewayWifiRepository.ConnectionState.Connected ->
            "Gateway connected - SnifferStatus updates about every 2s"
        GatewayWifiRepository.ConnectionState.Connecting -> "Connecting to gateway..."
        GatewayWifiRepository.ConnectionState.Reconnecting -> "Reconnecting to gateway..."
        is GatewayWifiRepository.ConnectionState.Error ->
            "Gateway: ${connection.message}"
        GatewayWifiRepository.ConnectionState.Disconnected ->
            "Gateway not connected - adopt a wolfPACK gateway for live discovery"
    }
