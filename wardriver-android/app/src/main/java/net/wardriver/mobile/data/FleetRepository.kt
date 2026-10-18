package net.wardriver.mobile.data

import android.content.Context
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.db.FleetRosterDao
import net.wardriver.mobile.db.FleetRosterEntity
import net.wardriver.mobile.wardriverApp

/** CRUD for phone-owned fleet roster; merges live SoftAP `SnifferStatus` rows. */
class FleetRepository(
    private val appContext: Context,
    private val dao: FleetRosterDao,
) {
    /** Fully adopted sniffers shown in the Fleet section. */
    val adoptedRosterFlow: Flow<List<FleetRosterEntity>> = dao.adoptedFlow()

    /** In-flight adopts (PolicySync pushed; waiting for sniffer-bound confirmation). */
    val adoptPendingFlow: Flow<List<FleetRosterEntity>> = dao.adoptPendingFlow()

    /** Last `SnifferStatus` batch (all rows, adopted or not) for Nearby / Adopting UI. */
    private val _nearbySniffers = MutableStateFlow<List<SnifferStatusRow>>(emptyList())
    val nearbySniffersFlow: StateFlow<List<SnifferStatusRow>> = _nearbySniffers.asStateFlow()

    /**
     * Start adoption: Room `adopt_pending=1`, push PolicySync (includes pending).
     * Fleet promotion waits for `adoption_state == ADOPTED` on live status.
     */
    suspend fun adopt(snifferId: Int, mac: String): AdoptResult {
        if (snifferId == 0) return AdoptResult.InvalidSnifferId
        val normalizedMac = normalizeMac(mac) ?: return AdoptResult.InvalidMac
        val existing = dao.get(snifferId)
        if (existing?.adopted == true) {
            return AdoptResult.AlreadyAdopted(snifferId)
        }
        val usedSlots =
            (dao.adopted().map { it.fleetSlot } + dao.adoptPending().map { it.fleetSlot })
                .distinct()
        val slot =
            existing?.fleetSlot
                ?: nextFreeFleetSlot(usedSlots)
                ?: return AdoptResult.RosterFull
        val statusRow = _nearbySniffers.value.find { it.snifferId == snifferId }
        val row =
            FleetRosterEntity(
                snifferId = snifferId,
                mac = normalizedMac,
                fleetSlot = slot,
                role = existing?.role ?: statusRow?.roleLabel() ?: ROLE_WIFI,
                band = existing?.band ?: DEFAULT_BAND,
                pinnedChannel = existing?.pinnedChannel,
                adopted = false,
                adoptPending = true,
                lastSeenMs = statusRow?.lastSeenMs ?: existing?.lastSeenMs,
                cpuLoadPct = statusRow?.cpuLoadPct ?: existing?.cpuLoadPct,
                heapAvailPct = statusRow?.heapAvailPct ?: existing?.heapAvailPct,
                rebootCount = statusRow?.rebootCount ?: existing?.rebootCount,
                state = statusRow?.state ?: existing?.state,
            )
        dao.upsert(row)
        val pushResult = PolicySyncAssembler.assembleAndPush(appContext)
        return AdoptResult.Pending(
            snifferId = snifferId,
            policyPushed = pushResult.succeeded,
        )
    }

    /** Cancel in-flight adopt or remove from fleet; push updated PolicySync. */
    suspend fun remove(snifferId: Int): RemoveResult {
        val row = dao.get(snifferId) ?: return RemoveResult.NotFound
        dao.delete(row)
        val pushResult = PolicySyncAssembler.assembleAndPush(appContext)
        return RemoveResult.Ok(snifferId = snifferId, policyPushed = pushResult.succeeded)
    }

    /** Clear all fleet rows when gateway reports a physical BOOT unbind. */
    suspend fun clearAll(): Int = dao.deleteAll()

    /** Re-arm gateway adoption retries by resending full PolicySync. */
    suspend fun retryAdopt(snifferId: Int): AdoptResult {
        if (snifferId == 0) return AdoptResult.InvalidSnifferId
        val existing = dao.get(snifferId) ?: return AdoptResult.NotFound
        if (!existing.adoptPending || existing.adopted) {
            return AdoptResult.NotFound
        }
        val pushResult = PolicySyncAssembler.assembleAndPush(appContext)
        return AdoptResult.Pending(
            snifferId = snifferId,
            policyPushed = pushResult.succeeded,
        )
    }

    /** Resend PolicySync when gateway ack is missing (bounded automatic recovery). */
    suspend fun resendPolicySyncForAdopt(): Boolean {
        if (!gatewayConnected()) return false
        return PolicySyncAssembler.assembleAndPush(appContext).succeeded
    }

    /** Set sniffer radio role (`wifi` or `ble`) and push policy_sync. */
    suspend fun setRole(snifferId: Int, role: String) {
        val normalized = role.lowercase()
        if (normalized != ROLE_WIFI && normalized != ROLE_BLE) return
        val existing = dao.get(snifferId) ?: return
        dao.upsert(existing.copy(role = normalized))
        PolicySyncAssembler.assembleAndPush(appContext)
    }

    /** Manual band assignment for WiFi sniffers (`2.4` or `5`); pushes channel plan sync. */
    suspend fun setBand(snifferId: Int, band: String) {
        if (band != BAND_24 && band != BAND_5) return
        val existing = dao.get(snifferId) ?: return
        if (existing.role != ROLE_WIFI) return
        dao.upsert(existing.copy(band = band))
        pushChannelPlanAfterFleetEdit()
    }

    /** Pin sniffer to one channel (`null` clears pin); pushes channel plan sync. */
    suspend fun setPinnedChannel(snifferId: Int, channel: Int?) {
        val existing = dao.get(snifferId) ?: return
        if (existing.role != ROLE_WIFI) return
        dao.upsert(existing.copy(pinnedChannel = channel?.coerceIn(1, 177)))
        pushChannelPlanAfterFleetEdit()
    }

    private suspend fun pushChannelPlanAfterFleetEdit() {
        if (gatewayConnected()) {
            ChannelPlanSyncAssembler.assembleAndPush(appContext)
        }
    }

    /**
     * Merge gateway `SnifferStatus` notify rows into roster telemetry.
     * Promotes `adopt_pending` rows when wire `adoption_state == ADOPTED`.
     */
    suspend fun mergeSnifferStatus(rows: List<SnifferStatusRow>) {
        _nearbySniffers.value = rows
        for (status in rows) {
            val existing = dao.get(status.snifferId) ?: continue
            val normalizedStatusMac = normalizeMac(status.mac) ?: continue
            val normalizedExistingMac = normalizeMac(existing.mac) ?: continue
            if (normalizedExistingMac != normalizedStatusMac) continue
            var next = existing
            if (existing.adoptPending &&
                !existing.adopted &&
                status.adoptionState == ADOPTION_ADOPTED
            ) {
                next =
                    existing.copy(
                        adopted = true,
                        adoptPending = false,
                    )
            }
            dao.upsert(
                next.copy(
                    mac = status.mac,
                    fleetSlot = if (status.fleetSlot > 0) status.fleetSlot else next.fleetSlot,
                    lastSeenMs = status.lastSeenMs,
                    cpuLoadPct = status.cpuLoadPct,
                    heapAvailPct = status.heapAvailPct,
                    rebootCount = status.rebootCount,
                    state = status.state,
                    band = status.assignedBandLabel().takeIf { it.isNotBlank() } ?: next.band,
                    pinnedChannel =
                        status.livePinnedChannel()?.takeIf { ch -> ch > 0 } ?: next.pinnedChannel,
                ),
            )
        }
    }

    /** PackLink `SnifferStatus` row decoded from JNI. */
    data class SnifferStatusRow(
        val snifferId: Int,
        val mac: String,
        val lastSeenMs: Long,
        val state: Int,
        val roleWire: Int,
        val cpuLoadPct: Int,
        val heapAvailPct: Int,
        val fleetSlot: Int,
        val rebootCount: Int,
        val adoptionState: Int = ADOPTION_EMPTY,
        val adoptProgress: Int = ADOPT_PROGRESS_NONE,
        val adoptReason: Int = ADOPT_REASON_NONE,
        val fiveGhzCapable: Int = 0,
        val assignedBand: Int = 0,
        val pinnedChannel: Int = 0,
        val assignedChanCount: Int = 0,
        val assignedChannelsPreview: List<Int> = emptyList(),
    ) {
        fun roleLabel(): String = if (roleWire == 1) ROLE_BLE else ROLE_WIFI

        fun assignedBandLabel(): String =
            when (assignedBand) {
                2 -> BAND_5
                1 -> BAND_24
                else -> ""
            }

        fun livePinnedChannel(): Int? = pinnedChannel.takeIf { it > 0 }

        fun adoptTimedOut(): Boolean =
            adoptProgress == FleetRepository.ADOPT_PROGRESS_TIMED_OUT_HELLO_LIVE ||
                adoptProgress == FleetRepository.ADOPT_PROGRESS_TIMED_OUT_NO_HELLO

        fun adoptRetryable(): Boolean = adoptTimedOut()

        fun adoptStatusLabel(gatewayConnected: Boolean, policyAckOk: Boolean): String =
            when {
                !gatewayConnected -> "Waiting for gateway connect"
                !policyAckOk -> "Waiting for gateway policy ack"
                adoptionState == FleetRepository.ADOPTION_ADOPTED ||
                    adoptProgress == FleetRepository.ADOPT_PROGRESS_CONFIRMED ->
                    "Bound confirmed - promoting"
                adoptProgress == FleetRepository.ADOPT_PROGRESS_RETRYING_WELCOME ->
                    "Retrying ControlWelcome"
                adoptProgress == FleetRepository.ADOPT_PROGRESS_WAITING_POLICY ->
                    "Waiting for gateway policy"
                adoptProgress == FleetRepository.ADOPT_PROGRESS_TIMED_OUT_HELLO_LIVE ->
                    "Bind timeout - sniffer still heard; remove from old gateway or clear NVS"
                adoptProgress == FleetRepository.ADOPT_PROGRESS_TIMED_OUT_NO_HELLO ->
                    "Bind timeout - sniffer not heard; check range and power"
                adoptReason == FleetRepository.ADOPT_REASON_SEND_FAILED ->
                    "Welcome send failed - gateway retrying"
                else -> "Waiting for gateway confirmation"
            }
    }

    private suspend fun gatewayConnected(): Boolean {
        val state = appContext.wardriverApp().gatewayWifiRepository.connectionState.first()
        return state is GatewayWifiRepository.ConnectionState.Connected
    }

    sealed class AdoptResult {
        data class Pending(val snifferId: Int, val policyPushed: Boolean) : AdoptResult()

        data class AlreadyAdopted(val snifferId: Int) : AdoptResult()

        data object RosterFull : AdoptResult()

        data object InvalidMac : AdoptResult()

        data object InvalidSnifferId : AdoptResult()

        data object NotFound : AdoptResult()
    }

    sealed class RemoveResult {
        data class Ok(val snifferId: Int, val policyPushed: Boolean) : RemoveResult()

        data object NotFound : RemoveResult()
    }

    companion object {
        const val ROLE_WIFI = "wifi"
        const val ROLE_BLE = "ble"
        const val BAND_24 = "2.4"
        const val BAND_5 = "5"
        const val DEFAULT_BAND = BAND_24
        const val FLEET_MAX_SNIFFERS = 48

        const val ADOPTION_EMPTY = 0
        const val ADOPTION_PENDING = 1
        const val ADOPTION_ADOPTED = 2
        const val ADOPTION_REMOVED = 3

        const val ADOPT_PROGRESS_NONE = 0
        const val ADOPT_PROGRESS_WAITING_POLICY = 1
        const val ADOPT_PROGRESS_RETRYING_WELCOME = 2
        const val ADOPT_PROGRESS_CONFIRMED = 3
        const val ADOPT_PROGRESS_TIMED_OUT_HELLO_LIVE = 4
        const val ADOPT_PROGRESS_TIMED_OUT_NO_HELLO = 5

        const val ADOPT_REASON_NONE = 0
        const val ADOPT_REASON_SEND_FAILED = 1
        const val ADOPT_REASON_POSSIBLE_EXISTING_BINDING = 2
        const val ADOPT_REASON_ROSTER_REMOVED = 3

        fun nextFreeFleetSlot(used: List<Int>): Int? {
            for (slot in 1..FLEET_MAX_SNIFFERS) {
                if (slot !in used) return slot
            }
            return null
        }

        /** Normalize MAC to upper-case colon hex (`AA:BB:CC:DD:EE:FF`). */
        fun normalizeMac(input: String): String? {
            val hex = input.replace(":", "").replace("-", "").trim().uppercase()
            if (hex.length != 12 || !hex.all { it in '0'..'9' || it in 'A'..'F' }) return null
            return (0 until 6).joinToString(":") { i -> hex.substring(i * 2, i * 2 + 2) }
        }
    }
}
