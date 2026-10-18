package net.wardriver.mobile.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.data.GatewayWifiRepository.CaptureStatus
import net.wardriver.mobile.data.GatewayWifiRepository.ConnectState
import net.wardriver.mobile.data.GatewayWifiRepository.ConnectionState
import net.wardriver.mobile.data.GatewayWifiRepository.GatewayInfo
import net.wardriver.mobile.data.GatewayWifiRepository.PendingTofu

/**
 * Owns the PackLink gateway-link [StateFlow] state.
 *
 * The repository reads and writes these flows through this class so that the
 * link state is testable in isolation without a real TCP socket.
 */
class GatewayLinkState {

    // ── backing MutableStateFlows ───────────────────────────────────────────
    private val _connectionState = MutableStateFlow<ConnectionState>(ConnectionState.Disconnected)
    private val _gatewayInfo = MutableStateFlow<GatewayInfo?>(null)
    private val _connectState = MutableStateFlow<ConnectState>(ConnectState.Idle)
    private val _pendingTofu = MutableStateFlow<PendingTofu?>(null)
    private val _captureStatus = MutableStateFlow<CaptureStatus?>(null)
    private val _snifferStatus = MutableStateFlow<List<FleetRepository.SnifferStatusRow>>(emptyList())
    private val _observationBulkRxCount = MutableStateFlow(0L)
    private val _lastPolicySyncAck = MutableStateFlow<NativeCore.PolicySyncAck?>(null)
    private val _lastChannelPlanSyncAck = MutableStateFlow<NativeCore.ChannelPlanSyncAck?>(null)

    // ── read-only StateFlows ────────────────────────────────────────────────
    val connectionState: StateFlow<ConnectionState> = _connectionState.asStateFlow()
    val gatewayInfo: StateFlow<GatewayInfo?> = _gatewayInfo.asStateFlow()
    val connectState: StateFlow<ConnectState> = _connectState.asStateFlow()
    val pendingTofu: StateFlow<PendingTofu?> = _pendingTofu.asStateFlow()
    val captureStatus: StateFlow<CaptureStatus?> = _captureStatus.asStateFlow()
    val snifferStatus: StateFlow<List<FleetRepository.SnifferStatusRow>> = _snifferStatus.asStateFlow()
    val observationBulkRxCount: StateFlow<Long> = _observationBulkRxCount.asStateFlow()
    val lastPolicySyncAck: StateFlow<NativeCore.PolicySyncAck?> = _lastPolicySyncAck.asStateFlow()
    val lastChannelPlanSyncAck: StateFlow<NativeCore.ChannelPlanSyncAck?> =
        _lastChannelPlanSyncAck.asStateFlow()

    // ── read helpers (used by repository internals) ─────────────────────────
    fun connectionStateValue(): ConnectionState = _connectionState.value
    fun gatewayInfoValue(): GatewayInfo? = _gatewayInfo.value
    fun pendingTofuValue(): PendingTofu? = _pendingTofu.value
    fun capabilitiesValue(): Int = (_gatewayInfo.value?.capabilities ?: 0)

    /**
     * Reads and nulls the pending TOFU, returning the claimed value or null.
     * The caller is responsible for holding the repository's [writerLock]
     * to preserve the original atomicity guarantee.
     */
    fun claimPendingTofu(): PendingTofu? {
        val claimed = _pendingTofu.value ?: return null
        _pendingTofu.value = null
        return claimed
    }

    // ── write helpers ───────────────────────────────────────────────────────
    fun setConnectionState(state: ConnectionState) { _connectionState.value = state }
    fun setGatewayInfo(info: GatewayInfo?) { _gatewayInfo.value = info }
    fun setConnectState(state: ConnectState) { _connectState.value = state }
    fun setPendingTofu(tofu: PendingTofu?) { _pendingTofu.value = tofu }
    fun setCaptureStatus(status: CaptureStatus?) { _captureStatus.value = status }
    fun setSnifferStatus(rows: List<FleetRepository.SnifferStatusRow>) { _snifferStatus.value = rows }
    fun incrementObservationBulkRxCount() { _observationBulkRxCount.value = _observationBulkRxCount.value + 1 }
    fun setObservationBulkRxCount(count: Long) { _observationBulkRxCount.value = count }
    fun setLastPolicySyncAck(ack: NativeCore.PolicySyncAck?) { _lastPolicySyncAck.value = ack }
    fun setLastChannelPlanSyncAck(ack: NativeCore.ChannelPlanSyncAck?) { _lastChannelPlanSyncAck.value = ack }
}
