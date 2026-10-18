package net.wardriver.mobile.ui.dashboard

import android.content.Context
import android.location.Location
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.data.GpsDetailHelpers
import net.wardriver.mobile.data.OneShotLocation
import net.wardriver.mobile.data.PolicySyncAssembler
import net.wardriver.mobile.data.StatsRepository
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.service.WardriveSessionStartState
import net.wardriver.mobile.wardriverApp

/** ViewModel for dashboard glance cards, fleet actions, and gateway Wi-Fi connect. */
class DashboardViewModel(
    private val appContext: Context,
    private val db: WardriverDatabase,
    private val gatewayWifi: GatewayWifiRepository,
    private val appPreferences: AppPreferences,
) : ViewModel() {
    private val _gpsLocation = MutableStateFlow<Location?>(null)
    val gpsLocation: StateFlow<Location?> = _gpsLocation.asStateFlow()

    val sessionStartState: StateFlow<WardriveSessionStartState> =
        WardriveSessionService.startStateFlow.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            WardriveSessionStartState.Idle,
        )

    /** Active wardrive session id updates when user starts/stops from dashboard. */
    val sessionIdState: StateFlow<Long?> =
        WardriveSessionService.sessionIdFlow.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            null,
        )

    private val _snackbarMessage = MutableStateFlow<String?>(null)
    val snackbarMessage: StateFlow<String?> = _snackbarMessage.asStateFlow()

    private val _preflightWarnings = MutableStateFlow<List<String>>(emptyList())
    val preflightWarnings: StateFlow<List<String>> = _preflightWarnings.asStateFlow()

    private val statsRepository =
        StatsRepository(
            db = db,
            gatewayWifi = gatewayWifi,
            appPreferences = appPreferences,
            gpsLocation = {
                GpsDetailHelpers.resolveDisplayLocation(
                    sessionActive = sessionIdState.value != null,
                    wardrive = WardriveSessionService.wardriveGpsFlow.value,
                    idle = _gpsLocation.value,
                )
            },
            activeSessionId = { sessionIdState.value },
            homeZoneLatchSuppressed = { WardriveSessionService.homeZoneSuppressedFlow.value },
        )

    val stats: StateFlow<StatsRepository.DashboardStats> = statsRepository.stats

    init {
        refreshGps()
        refreshPreflightWarnings()
        statsRepository.startPolling(viewModelScope)
        viewModelScope.launch {
            combine(sessionIdState, WardriveSessionService.wardriveGpsFlow) { sessionId, wardriveLoc ->
                sessionId to wardriveLoc
            }.collect { (sessionId, wardriveLoc) ->
                if (sessionId != null && wardriveLoc != null) {
                    _gpsLocation.value = wardriveLoc
                }
            }
        }
        viewModelScope.launch {
            sessionStartState.collect { state ->
                if (state is WardriveSessionStartState.Failed) {
                    _snackbarMessage.value = state.reason
                }
            }
        }
    }

    override fun onCleared() {
        statsRepository.stopPolling()
        super.onCleared()
    }

    fun refreshPreflightWarnings() {
        viewModelScope.launch {
            _preflightWarnings.value = WardriveStartPreflight.evaluate(appContext).warnings
        }
    }

    /** One-shot location, gated on the persisted [LocationSource] (fused opt-in vs GPS-only). */
    fun refreshGps() {
        viewModelScope.launch {
            OneShotLocation.resolve(appContext)?.let { _gpsLocation.value = it }
        }
    }

    fun startWardrive() {
        viewModelScope.launch {
            refreshPreflightWarnings()
            val preflight = WardriveStartPreflight.evaluate(appContext)
            if (!preflight.canStart) {
                _snackbarMessage.value = preflight.hardBlockReason ?: "Cannot start wardrive"
                return@launch
            }
            if (!WardriveSessionService.start(appContext)) {
                val failed = WardriveSessionService.startStateFlow.value
                if (failed is WardriveSessionStartState.Failed) {
                    _snackbarMessage.value = failed.reason
                }
            }
        }
    }

    fun clearSnackbar() {
        _snackbarMessage.value = null
    }

    fun clearFailedStart() {
        WardriveSessionService.clearFailedStart()
        clearSnackbar()
    }

    fun stopWardrive() {
        WardriveSessionService.stop(appContext)
    }

    /** Pause fleet scanning — mirrors web `/api/mode` management. */
    fun pauseFleet() {
        viewModelScope.launch {
            appPreferences.setScanAllowed(false)
            PolicySyncAssembler.assembleAndPush(appContext)
            statsRepository.refresh()
        }
    }

    /** Resume wardrive scanning — mirrors web `/api/mode` wardrive. */
    fun resumeFleetWardrive() {
        viewModelScope.launch {
            appPreferences.setScanAllowed(true)
            PolicySyncAssembler.assembleAndPush(appContext)
            statsRepository.refresh()
        }
    }

    fun connectGateway() {
        gatewayWifi.connectPreferred()
    }

    class Factory(
        private val appContext: Context,
    ) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            val app = appContext.wardriverApp()
            return DashboardViewModel(
                appContext = appContext.applicationContext,
                db = app.database,
                gatewayWifi = app.gatewayWifiRepository,
                appPreferences = app.appPreferences,
            ) as T
        }
    }
}
