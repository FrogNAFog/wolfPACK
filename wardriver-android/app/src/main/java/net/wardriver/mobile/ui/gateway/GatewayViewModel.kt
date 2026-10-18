package net.wardriver.mobile.ui.gateway

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.data.PolicySyncAssembler
import net.wardriver.mobile.ui.WifiPermissionNotifier
import net.wardriver.mobile.wardriverApp

/** Thin wrapper over [GatewayWifiRepository] for dashboard and gateway settings screens. */
class GatewayViewModel(
    private val appContext: Context,
    private val gatewayWifi: GatewayWifiRepository,
    private val appPreferences: AppPreferences,
) : ViewModel() {
    val connectionState: StateFlow<GatewayWifiRepository.ConnectionState> =
        gatewayWifi.connectionState.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            GatewayWifiRepository.ConnectionState.Disconnected,
        )

    val connectState: StateFlow<GatewayWifiRepository.ConnectState> =
        gatewayWifi.connectState.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            GatewayWifiRepository.ConnectState.Idle,
        )

    val pendingTofu: StateFlow<GatewayWifiRepository.PendingTofu?> =
        gatewayWifi.pendingTofu.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            null,
        )

    val gatewayInfo: StateFlow<GatewayWifiRepository.GatewayInfo?> =
        gatewayWifi.gatewayInfoFlow.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            null,
        )

    val softApSsid: StateFlow<String> =
        appPreferences.gatewaySoftApSsidFlow.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            AppPreferences.DEFAULT_GATEWAY_SOFTAP_SSID,
        )

    val softApPassword: StateFlow<String> =
        appPreferences.gatewaySoftApPasswordFlow.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            AppPreferences.DEFAULT_GATEWAY_SOFTAP_PASS,
        )

    val hasTrustedGateway: StateFlow<Boolean> =
        combine(connectionState, WifiPermissionNotifier.tick) { _, _ ->
            gatewayWifi.hasTrustedGateway()
        }.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            gatewayWifi.hasTrustedGateway(),
        )

    private val _connectSheetOpen = MutableStateFlow(false)
    val connectSheetOpen: StateFlow<Boolean> = _connectSheetOpen.asStateFlow()

    fun canConnect(): Boolean = gatewayWifi.canConnect()

    fun connect() {
        gatewayWifi.connect()
    }

    fun confirmPendingTofu() {
        gatewayWifi.confirmPendingTofu()
    }

    fun cancelPendingTofu() {
        gatewayWifi.cancelPendingTofu()
    }

    fun connectPreferred() {
        gatewayWifi.connectPreferred()
    }

    fun disconnect() {
        gatewayWifi.disconnect()
    }

    fun forgetGateway() {
        gatewayWifi.forgetGateway()
    }

    /** Desk / settings mirror of gateway physical BOOT unbind (clears Room + pushes empty PolicySync). */
    fun clearFleetBinding() {
        viewModelScope.launch {
            appContext.wardriverApp().fleetRepository.clearAll()
            PolicySyncAssembler.pushIfConnected(appContext)
        }
    }

    fun openConnectSheet() {
        _connectSheetOpen.value = true
    }

    fun closeConnectSheet() {
        _connectSheetOpen.value = false
    }

    /** Sheet swipe/back: cancel in-progress TOFU only; keep an established PackLink session. */
    fun dismissConnectSheet() {
        if (pendingTofu.value != null) {
            cancelPendingTofu()
        } else {
            closeConnectSheet()
        }
    }

    fun saveSoftApCreds(ssid: String, password: String, pushToGateway: Boolean) {
        viewModelScope.launch {
            appPreferences.saveGatewaySoftApCreds(ssid, password)
            if (pushToGateway && connectionState.value is GatewayWifiRepository.ConnectionState.Connected) {
                gatewayWifi.pushSoftApConfig(ssid, password)
            }
        }
    }

    class Factory(
        private val appContext: Context,
    ) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            val app = appContext.wardriverApp()
            return GatewayViewModel(
                appContext = appContext.applicationContext,
                gatewayWifi = app.gatewayWifiRepository,
                appPreferences = app.appPreferences,
            ) as T
        }
    }
}
