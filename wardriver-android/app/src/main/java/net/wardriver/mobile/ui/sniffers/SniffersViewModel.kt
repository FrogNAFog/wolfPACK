package net.wardriver.mobile.ui.sniffers

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.FleetRepository
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.db.FleetRosterEntity
import net.wardriver.mobile.wardriverApp

/** ViewModel for Sniffers roster; subscribes to PackLink `SnifferStatus`. */
class SniffersViewModel(
    private val fleetRepository: FleetRepository,
    private val gatewayWifi: GatewayWifiRepository,
    private val appPreferences: AppPreferences,
) : ViewModel() {
    private val _uiState = MutableStateFlow(SniffersUiState())
    val uiState: StateFlow<SniffersUiState> = _uiState.asStateFlow()

    init {
        viewModelScope.launch {
            combine(
                combine(
                    fleetRepository.adoptedRosterFlow,
                    fleetRepository.adoptPendingFlow,
                    gatewayWifi.snifferStatusFlow,
                    gatewayWifi.connectionState,
                ) { adopted, pendingRows, nearby, connection ->
                    Quadruple(adopted, pendingRows, nearby, connection)
                },
                gatewayWifi.lastPolicySyncAck,
            ) { rosterQuad, lastPolicyAck ->
                val (adopted, pendingRows, nearby, connection) = rosterQuad
                val fleetIds = adopted.map { it.snifferId }.toSet()
                val pendingIds = pendingRows.map { it.snifferId }.toSet()
                val connected =
                    connection is GatewayWifiRepository.ConnectionState.Connected
                val nowMs =
                    if (connected) {
                        gatewayWifi.extrapolatedGatewayNowMs()
                    } else {
                        0L
                    }
                val nearbyOnly =
                    filterPendingNearby(
                        nearby = nearby,
                        adoptedIds = fleetIds,
                        pendingIds = pendingIds,
                        gatewayNowMs = nowMs,
                        gatewayConnected = connected,
                    )
                SniffersCombine(
                    adopted = adopted,
                    pending = pendingRows,
                    nearby = nearby,
                    nearbyOnly = nearbyOnly,
                    connection = connection,
                    gatewayNowMs = nowMs,
                    lastPolicyAck = lastPolicyAck,
                )
            }.collect { c ->
                _uiState.update {
                    it.copy(
                        adoptedRoster = c.adopted,
                        adoptPending = c.pending,
                        nearbySniffers = c.nearby,
                        pendingNearby = c.nearbyOnly,
                        connection = c.connection,
                        gatewayNowMs = c.gatewayNowMs,
                        lastPolicyAckOk = c.lastPolicyAck?.result == 0,
                        lastPolicyAckSeq = c.lastPolicyAck?.policySeq,
                    )
                }
            }
        }
        viewModelScope.launch {
            appPreferences.channelPlanFlow.collect { plan ->
                _uiState.update { it.copy(channelPlan = plan) }
            }
        }
        viewModelScope.launch {
            while (isActive) {
                val connected =
                    gatewayWifi.connectionState.value is
                        GatewayWifiRepository.ConnectionState.Connected
                val nowMs =
                    if (connected) {
                        gatewayWifi.extrapolatedGatewayNowMs()
                    } else {
                        0L
                    }
                _uiState.update { state ->
                    val fleetIds = state.adoptedRoster.map { it.snifferId }.toSet()
                    val pendingIds = state.adoptPending.map { it.snifferId }.toSet()
                    val nearbyOnly =
                        filterPendingNearby(
                            nearby = state.nearbySniffers,
                            adoptedIds = fleetIds,
                            pendingIds = pendingIds,
                            gatewayNowMs = nowMs,
                            gatewayConnected = connected,
                        )
                    state.copy(gatewayNowMs = nowMs, pendingNearby = nearbyOnly)
                }
                refreshAdoptTimeouts(nowMs)
                maybeAutoResendPolicySync()
                delay(1_000L)
            }
        }
    }

    /** Connect to adopted gateway when Sniffers screen opens. */
    fun ensureGatewayConnected() {
        if (gatewayWifi.connectionState.value is GatewayWifiRepository.ConnectionState.Disconnected) {
            gatewayWifi.connectPreferred()
        }
    }

    fun adopt(snifferId: Int, mac: String) {
        viewModelScope.launch {
            when (val result = fleetRepository.adopt(snifferId, mac)) {
                is FleetRepository.AdoptResult.Pending -> {
                    _adoptStartedAtMs[result.snifferId] = System.currentTimeMillis()
                    _adoptFailedIds.remove(result.snifferId)
                    _policyAutoResendCount[result.snifferId] = 0
                    val suffix =
                        if (result.policyPushed) {
                            " - waiting for gateway confirmation"
                        } else {
                            " - connect gateway to push roster"
                        }
                    showSnackbar("Adopting sniffer ${shortSnifferId(result.snifferId)}$suffix")
                }
                is FleetRepository.AdoptResult.AlreadyAdopted ->
                    showSnackbar("Sniffer ${shortSnifferId(result.snifferId)} already in fleet")
                FleetRepository.AdoptResult.RosterFull ->
                    showSnackbar("Fleet roster full (${FleetRepository.FLEET_MAX_SNIFFERS} sniffers)")
                FleetRepository.AdoptResult.InvalidMac ->
                    showSnackbar("Invalid sniffer MAC")
                FleetRepository.AdoptResult.InvalidSnifferId ->
                    showSnackbar("Invalid sniffer ID")
                FleetRepository.AdoptResult.NotFound -> Unit
            }
        }
    }

    fun cancelAdopt(snifferId: Int) {
        viewModelScope.launch {
            when (val result = fleetRepository.remove(snifferId)) {
                is FleetRepository.RemoveResult.Ok -> {
                    _adoptStartedAtMs.remove(snifferId)
                    _adoptFailedIds.remove(snifferId)
                    _policyAutoResendCount.remove(snifferId)
                    val suffix =
                        if (result.policyPushed) {
                            ""
                        } else {
                            " - connect gateway to push roster"
                        }
                    showSnackbar("Cancelled adopt ${shortSnifferId(result.snifferId)}$suffix")
                }
                FleetRepository.RemoveResult.NotFound -> Unit
            }
        }
    }

    fun retryAdopt(snifferId: Int) {
        viewModelScope.launch {
            when (val result = fleetRepository.retryAdopt(snifferId)) {
                is FleetRepository.AdoptResult.Pending -> {
                    _adoptStartedAtMs[result.snifferId] = System.currentTimeMillis()
                    _adoptFailedIds.remove(result.snifferId)
                    _policyAutoResendCount[result.snifferId] = 0
                    val suffix =
                        if (result.policyPushed) {
                            " - retrying bind"
                        } else {
                            " - connect gateway to push roster"
                        }
                    showSnackbar("Retrying adopt ${shortSnifferId(result.snifferId)}$suffix")
                }
                FleetRepository.AdoptResult.NotFound -> Unit
                FleetRepository.AdoptResult.InvalidSnifferId ->
                    showSnackbar("Invalid sniffer ID")
                is FleetRepository.AdoptResult.AlreadyAdopted ->
                    showSnackbar("Sniffer ${shortSnifferId(result.snifferId)} already in fleet")
                FleetRepository.AdoptResult.RosterFull ->
                    showSnackbar("Fleet roster full (${FleetRepository.FLEET_MAX_SNIFFERS} sniffers)")
                FleetRepository.AdoptResult.InvalidMac ->
                    showSnackbar("Invalid sniffer MAC")
            }
        }
    }

    fun remove(snifferId: Int) {
        viewModelScope.launch {
            when (val result = fleetRepository.remove(snifferId)) {
                is FleetRepository.RemoveResult.Ok -> {
                    _adoptStartedAtMs.remove(snifferId)
                    val suffix =
                        if (result.policyPushed) {
                            ""
                        } else {
                            " - connect gateway to push roster"
                        }
                    showSnackbar("Removed sniffer ${shortSnifferId(result.snifferId)}$suffix")
                }
                FleetRepository.RemoveResult.NotFound -> Unit
            }
        }
    }

    fun setRole(snifferId: Int, role: String) {
        viewModelScope.launch {
            fleetRepository.setRole(snifferId, role)
        }
    }

    fun setBand(snifferId: Int, band: String) {
        viewModelScope.launch {
            fleetRepository.setBand(snifferId, band)
        }
    }

    fun setPinnedChannel(snifferId: Int, channel: Int?) {
        viewModelScope.launch {
            fleetRepository.setPinnedChannel(snifferId, channel)
        }
    }

    fun clearSnackbar() {
        _uiState.update { it.copy(snackbarMessage = null) }
    }

    private fun showSnackbar(message: String) {
        _uiState.update { it.copy(snackbarMessage = message) }
    }

    private fun refreshAdoptTimeouts(gatewayNowMs: Long) {
        val pending = _uiState.value.adoptPending
        if (pending.isEmpty()) return
        val liveById = _uiState.value.nearbySniffers.associateBy { it.snifferId }
        val failedIds = mutableSetOf<Int>()
        for (row in pending) {
            val live = liveById[row.snifferId]
            if (live?.adoptRetryable() == true) {
                failedIds.add(row.snifferId)
            }
        }
        if (failedIds != _adoptFailedIds) {
            _adoptFailedIds.clear()
            _adoptFailedIds.addAll(failedIds)
            _uiState.update { it.copy(adoptFailedIds = failedIds.toSet()) }
        }
        if (gatewayNowMs < 0L) return
    }

    private fun maybeAutoResendPolicySync() {
        val pending = _uiState.value.adoptPending
        if (pending.isEmpty()) return
        if (_uiState.value.lastPolicyAckOk) return
        val now = System.currentTimeMillis()
        for (row in pending) {
            val count = _policyAutoResendCount[row.snifferId] ?: 0
            if (count >= POLICY_AUTO_RESEND_MAX) continue
            val last = _policyAutoResendAtMs[row.snifferId] ?: 0L
            if (now - last < POLICY_AUTO_RESEND_INTERVAL_MS) continue
            _policyAutoResendAtMs[row.snifferId] = now
            _policyAutoResendCount[row.snifferId] = count + 1
            viewModelScope.launch {
                fleetRepository.resendPolicySyncForAdopt()
            }
        }
    }

    private val _adoptStartedAtMs = mutableMapOf<Int, Long>()
    private val _adoptFailedIds = mutableSetOf<Int>()
    private val _policyAutoResendCount = mutableMapOf<Int, Int>()
    private val _policyAutoResendAtMs = mutableMapOf<Int, Long>()

    class Factory(private val appContext: Context) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            val app = appContext.wardriverApp()
            return SniffersViewModel(
                fleetRepository = app.fleetRepository,
                gatewayWifi = app.gatewayWifiRepository,
                appPreferences = app.appPreferences,
            ) as T
        }
    }

    companion object {
        private const val POLICY_AUTO_RESEND_MAX = 3
        private const val POLICY_AUTO_RESEND_INTERVAL_MS = 5_000L
    }
}

private data class Quadruple<A, B, C, D>(
    val first: A,
    val second: B,
    val third: C,
    val fourth: D,
)

private data class SniffersCombine(
    val adopted: List<FleetRosterEntity>,
    val pending: List<FleetRosterEntity>,
    val nearby: List<FleetRepository.SnifferStatusRow>,
    val nearbyOnly: List<FleetRepository.SnifferStatusRow>,
    val connection: GatewayWifiRepository.ConnectionState,
    val gatewayNowMs: Long,
    val lastPolicyAck: net.wardriver.mobile.NativeCore.PolicySyncAck?,
)

data class SniffersUiState(
    val adoptedRoster: List<FleetRosterEntity> = emptyList(),
    val adoptPending: List<FleetRosterEntity> = emptyList(),
    val nearbySniffers: List<FleetRepository.SnifferStatusRow> = emptyList(),
    val pendingNearby: List<FleetRepository.SnifferStatusRow> = emptyList(),
    val channelPlan: AppPreferences.ChannelPlan =
        AppPreferences.ChannelPlan(
            dwellMs = AppPreferences.DEFAULT_DWELL_MS,
            channels24 = AppPreferences.DEFAULT_CHANNELS_24,
            channels5 = AppPreferences.DEFAULT_CHANNELS_5,
            channels5Dfs = AppPreferences.DEFAULT_CHANNELS_5_DFS,
        ),
    val connection: GatewayWifiRepository.ConnectionState =
        GatewayWifiRepository.ConnectionState.Disconnected,
    val gatewayNowMs: Long = 0L,
    val lastPolicyAckOk: Boolean = false,
    val lastPolicyAckSeq: Long? = null,
    val adoptFailedIds: Set<Int> = emptySet(),
    val snackbarMessage: String? = null,
)
