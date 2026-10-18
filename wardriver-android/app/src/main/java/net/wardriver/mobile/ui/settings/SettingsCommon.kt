package net.wardriver.mobile.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.data.PolicySyncAssembler
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Shared settings sub-screen shell with back nav and optional pending-sync chip. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsSubScreen(
    title: String,
    onNavigateUp: () -> Unit,
    showPendingGatewayChip: Boolean = true,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val context = LocalContext.current
    val gatewayWifi = context.wardriverApp().gatewayWifiRepository
    val connection by gatewayWifi.connectionState.collectAsState()
    val connected = connection is GatewayWifiRepository.ConnectionState.Connected
    val scope = rememberCoroutineScope()

    Scaffold(
        modifier = modifier,
        containerColor = PackColor.Background,
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onNavigateUp) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (showPendingGatewayChip && !connected) {
                        AssistChip(
                            onClick = {
                                scope.launch {
                                    gatewayWifi.connectPreferred()
                                }
                            },
                            label = { Text("Pending gateway sync") },
                            modifier = Modifier.padding(end = 8.dp),
                            colors =
                                AssistChipDefaults.assistChipColors(
                                    containerColor = PackColor.HotWash,
                                    labelColor = PackColor.Sun,
                                ),
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = PackColor.Header,
                        titleContentColor = PackColor.Text,
                        navigationIconContentColor = PackColor.Cyan,
                    ),
            )
        },
    ) { innerPadding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding)
                    .verticalScroll(rememberScrollState())
                    .padding(16.dp),
            content = content,
        )
    }
}

/** Persist phone-local DataStore prefs without gateway policy_sync. */
fun saveLocalPref(scope: CoroutineScope, block: suspend () -> Unit) {
    scope.launch { block() }
}

/** Save helper: persist then push policy_sync when gateway Wi-Fi is connected. */
fun saveAndSyncPolicy(scope: kotlinx.coroutines.CoroutineScope, context: android.content.Context, block: suspend () -> Unit) {
    scope.launch {
        block()
        PolicySyncAssembler.pushIfConnected(context)
        WardriveSessionService.syncDenylistsToActiveIngestor(context)
    }
}
