package net.wardriver.mobile.ui.settings.channelplan

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.theme.PackColor

/** Dwell, band pools, and hop schedule; pushes ChannelPlanSync or legacy PolicySync. */
@Composable
fun ChannelPlanScreen(
    onNavigateUp: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: ChannelPlanViewModel =
        viewModel(factory = ChannelPlanViewModel.Factory(LocalContext.current)),
) {
    val uiState by viewModel.uiState.collectAsState()
    val gatewayConnected = uiState.connection is GatewayWifiRepository.ConnectionState.Connected

    LaunchedEffect(Unit) {
        viewModel.ensureGatewayConnected()
    }

    SettingsSubScreen(
        title = "Channel plan",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = true,
        modifier = modifier,
    ) {
        if (!gatewayConnected) {
            Text(
                text =
                    "Gateway not connected. Save stores the plan on this phone; " +
                        "push happens when the gateway connects.",
                style = MaterialTheme.typography.labelMedium,
                color = PackColor.Sun,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .background(PackColor.HotWash)
                        .padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }
        ChannelPlanEditor(
            plan = uiState.channelPlan,
            channelPlanAckOk = uiState.lastChannelPlanAckOk,
            channelPlanAckRevision = uiState.lastChannelPlanAckRevision,
            hopCycleMs = uiState.hopCycleMs,
            unassignedCount = uiState.unassignedCount,
            supportsDedicatedSync = uiState.supportsChannelPlanSync,
            onSave = viewModel::saveChannelPlan,
            onResetDefaults = viewModel::resetChannelPlanDefaults,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}
