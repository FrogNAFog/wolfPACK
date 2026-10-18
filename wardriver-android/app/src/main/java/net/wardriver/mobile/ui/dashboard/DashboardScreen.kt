package net.wardriver.mobile.ui.dashboard

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
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
import net.wardriver.mobile.service.WardriveSessionStartState
import net.wardriver.mobile.ui.components.PackAlertBanner
import net.wardriver.mobile.ui.components.PackCard
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.components.PackSectionLabel
import net.wardriver.mobile.ui.components.PackSnackbarHost
import net.wardriver.mobile.ui.gateway.GatewayConnectSheet
import net.wardriver.mobile.ui.gateway.GatewayViewModel

/** Live fleet dashboard — parity with control web `/` glance cards and fleet mode actions. */
@Composable
fun DashboardScreen(
    modifier: Modifier = Modifier,
    onNavigateGatewaySettings: () -> Unit = {},
    viewModel: DashboardViewModel =
        viewModel(
            factory = DashboardViewModel.Factory(LocalContext.current),
        ),
    gatewayViewModel: GatewayViewModel =
        viewModel(
            factory = GatewayViewModel.Factory(LocalContext.current),
        ),
) {
    val stats by viewModel.stats.collectAsState()
    val hasTrusted by gatewayViewModel.hasTrustedGateway.collectAsState()
    val connectOpen by gatewayViewModel.connectSheetOpen.collectAsState()
    val sessionId by viewModel.sessionIdState.collectAsState()
    val sessionStartState by viewModel.sessionStartState.collectAsState()
    val preflightWarnings by viewModel.preflightWarnings.collectAsState()
    val snackbarMessage by viewModel.snackbarMessage.collectAsState()
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(snackbarMessage) {
        val message = snackbarMessage ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        viewModel.clearSnackbar()
    }

    Scaffold(
        modifier = modifier,
        snackbarHost = { PackSnackbarHost(snackbarHostState) },
        containerColor = androidx.compose.ui.graphics.Color.Transparent,
    ) { innerPadding ->
        LazyColumn(
            modifier = Modifier.fillMaxSize().padding(innerPadding),
            contentPadding = PaddingValues(vertical = 8.dp),
        ) {
            item {
                PackSectionLabel(
                    text = "Live stats",
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
            }

            stats.alertBanner?.let { banner ->
                item {
                    PackAlertBanner(
                        headline = banner.headline,
                        subtitle = banner.subtitle,
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
                    )
                }
            }

            item {
                DashboardGlanceGrid(stats = stats)
            }

            if (stats.gatewayInfo != null) {
                item {
                    WolfpackHealthCard(stats = stats)
                }
            }

            item {
                GatewayWifiCard(
                    connectionState = stats.gatewayWifiState,
                    hasTrustedGateway = hasTrusted,
                    onAdopt = gatewayViewModel::openConnectSheet,
                    onConnect = viewModel::connectGateway,
                    onDisconnect = gatewayViewModel::disconnect,
                    onManage = onNavigateGatewaySettings,
                )
            }

            item {
                LastAlertCard(detection = stats.lastDetection)
            }

            item {
                FleetModeCard(
                    scanAllowed = stats.scanAllowed,
                    sessionActive = sessionStartState.isRunning,
                    onPause = viewModel::pauseFleet,
                    onWardrive = viewModel::resumeFleetWardrive,
                )
            }

            item {
                WardriveSessionCard(
                    sessionStartState = sessionStartState,
                    sessionId = sessionId,
                    preflightWarnings = preflightWarnings,
                    onStart = viewModel::startWardrive,
                    onStop = viewModel::stopWardrive,
                )
            }

            if (stats.firmwareLine.isNotEmpty()) {
                item {
                    Text(
                        text = stats.firmwareLine,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }
        }
    }

    if (connectOpen) {
        GatewayConnectSheet(
            viewModel = gatewayViewModel,
        )
    }
}

@Composable
private fun FleetModeCard(
    scanAllowed: Boolean,
    sessionActive: Boolean,
    onPause: () -> Unit,
    onWardrive: () -> Unit,
    modifier: Modifier = Modifier,
) {
    PackCard(
        modifier = modifier.padding(horizontal = 12.dp, vertical = 4.dp),
    ) {
        PackSectionLabel("Fleet mode")
        Text(
            text =
                "Pause stops sniffer scanning. Wardrive resumes scanning; " +
                    "observation logging additionally needs GPS, time sync, and " +
                    "(if configured) leaving the home zone.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(top = 4.dp),
        )
        Row(modifier = Modifier.padding(top = 8.dp)) {
            PackOutlinedButton(
                text = "Pause",
                onClick = onPause,
                enabled = scanAllowed,
                modifier = Modifier.padding(end = 8.dp),
            )
            PackPrimaryButton(
                text = "Wardrive",
                onClick = onWardrive,
            )
        }
    }
}

@Composable
private fun WardriveSessionCard(
    sessionStartState: WardriveSessionStartState,
    sessionId: Long?,
    preflightWarnings: List<String>,
    onStart: () -> Unit,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val statusText =
        when (sessionStartState) {
            is WardriveSessionStartState.Running ->
                "Session ${sessionStartState.sessionId} running"
            WardriveSessionStartState.Starting -> "Starting session..."
            is WardriveSessionStartState.Failed -> sessionStartState.reason
            WardriveSessionStartState.Idle ->
                if (sessionId != null) {
                    "Session $sessionId running"
                } else {
                    "No active wardrive session"
                }
        }
    val canStart = sessionStartState.canStart
    val isRunning = sessionStartState.isRunning || sessionId != null

    PackCard(
        modifier = modifier.padding(horizontal = 12.dp, vertical = 4.dp),
    ) {
        PackSectionLabel("Wardrive session")
        Text(
            text = statusText,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(top = 4.dp),
        )
        if (preflightWarnings.isNotEmpty() && !isRunning) {
            Text(
                text = preflightWarnings.joinToString(" · "),
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        Row(modifier = Modifier.padding(top = 8.dp)) {
            PackPrimaryButton(
                text = if (sessionStartState is WardriveSessionStartState.Starting) "Starting..." else "Start wardrive",
                onClick = onStart,
                enabled = canStart && sessionStartState !is WardriveSessionStartState.Starting,
                modifier = Modifier.padding(end = 8.dp),
            )
            PackOutlinedButton(
                text = "Stop wardrive",
                onClick = onStop,
                enabled = isRunning,
            )
        }
    }
}
