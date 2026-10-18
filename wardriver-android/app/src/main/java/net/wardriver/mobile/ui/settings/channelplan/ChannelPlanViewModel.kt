package net.wardriver.mobile.ui.settings.channelplan

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.ChannelPlanSyncAssembler
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.data.PolicySyncAssembler
import net.wardriver.mobile.wardriverApp

/** Channel plan editor state, save/reset, and gateway ChannelPlanSync ack feedback. */
class ChannelPlanViewModel(
    private val appContext: Context,
    private val gatewayWifi: GatewayWifiRepository,
    private val appPreferences: AppPreferences,
) : ViewModel() {
    private val _uiState = MutableStateFlow(ChannelPlanUiState())
    val uiState: StateFlow<ChannelPlanUiState> = _uiState.asStateFlow()

    init {
        viewModelScope.launch {
            combine(
                gatewayWifi.connectionState,
                gatewayWifi.lastChannelPlanSyncAck,
                gatewayWifi.gatewayInfoFlow,
            ) { connection, lastChannelPlanAck, gatewayInfo ->
                ChannelPlanGatewaySlice(
                    connection = connection,
                    lastChannelPlanAck = lastChannelPlanAck,
                    supportsChannelPlanSync =
                        (gatewayInfo?.capabilities ?: 0) and GATEWAY_CAP_CHANNEL_PLAN_SYNC != 0,
                )
            }.collect { slice ->
                _uiState.update {
                    it.copy(
                        connection = slice.connection,
                        lastChannelPlanAckOk =
                            slice.lastChannelPlanAck?.result?.let { r -> r == 0 },
                        lastChannelPlanAckRevision = slice.lastChannelPlanAck?.revision,
                        hopCycleMs = slice.lastChannelPlanAck?.hopCycleMs,
                        unassignedCount = slice.lastChannelPlanAck?.unassignedCount,
                        supportsChannelPlanSync = slice.supportsChannelPlanSync,
                    )
                }
            }
        }
        viewModelScope.launch {
            appPreferences.channelPlanFlow.collect { plan ->
                _uiState.update { it.copy(channelPlan = plan) }
            }
        }
    }

    fun ensureGatewayConnected() {
        if (gatewayWifi.connectionState.value is GatewayWifiRepository.ConnectionState.Disconnected) {
            gatewayWifi.connectPreferred()
        }
    }

    fun saveChannelPlan(plan: AppPreferences.ChannelPlan) {
        viewModelScope.launch {
            val revision = appPreferences.incrementRevision()
            val toSave = plan.copy(revision = revision)
            appPreferences.saveChannelPlan(toSave)
            if (gatewayWifi.supportsChannelPlanSync()) {
                PolicySyncAssembler.pushIfConnected(appContext)
            } else {
                PolicySyncAssembler.assembleAndPush(appContext)
            }
        }
    }

    fun resetChannelPlanDefaults() {
        viewModelScope.launch {
            appPreferences.resetChannelPlanDefaults()
            ChannelPlanSyncAssembler.assembleAndPush(appContext)
        }
    }

    class Factory(private val appContext: Context) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            val app = appContext.wardriverApp()
            return ChannelPlanViewModel(
                appContext = appContext.applicationContext,
                gatewayWifi = app.gatewayWifiRepository,
                appPreferences = app.appPreferences,
            ) as T
        }
    }

    companion object {
        private const val GATEWAY_CAP_CHANNEL_PLAN_SYNC = 1 shl 0
    }
}

private data class ChannelPlanGatewaySlice(
    val connection: GatewayWifiRepository.ConnectionState,
    val lastChannelPlanAck: net.wardriver.mobile.NativeCore.ChannelPlanSyncAck?,
    val supportsChannelPlanSync: Boolean,
)

data class ChannelPlanUiState(
    val channelPlan: AppPreferences.ChannelPlan =
        AppPreferences.ChannelPlan(
            dwellMs = AppPreferences.DEFAULT_DWELL_MS,
            channels24 = AppPreferences.DEFAULT_CHANNELS_24,
            channels5 = AppPreferences.DEFAULT_CHANNELS_5,
            channels5Dfs = AppPreferences.DEFAULT_CHANNELS_5_DFS,
        ),
    val connection: GatewayWifiRepository.ConnectionState =
        GatewayWifiRepository.ConnectionState.Disconnected,
    val lastChannelPlanAckOk: Boolean? = null,
    val lastChannelPlanAckRevision: Long? = null,
    val hopCycleMs: Long? = null,
    val unassignedCount: Int? = null,
    val supportsChannelPlanSync: Boolean = false,
)
