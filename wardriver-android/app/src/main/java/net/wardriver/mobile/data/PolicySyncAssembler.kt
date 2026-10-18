package net.wardriver.mobile.data

import android.content.Context
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.policy.PolicySyncBuilder
import net.wardriver.mobile.wardriverApp

/**
 * Merges [AppPreferences], fleet roster, and geofence into PackLink `PolicySyncV1` wire bytes.
 * Call after any settings or roster change when the gateway Wi-Fi link is connected.
 */
object PolicySyncAssembler {
    /** Build policy_sync wire bytes from current DataStore snapshot and optional roster. */
    suspend fun buildWireBytes(
        context: Context,
        roster: List<PolicySyncBuilder.RosterEntry> = emptyList(),
        wardriveSuppressed: Boolean = false,
        seq: Int = 0,
    ): ByteArray? {
        val snap = context.wardriverApp().appPreferences.snapshot()
        val geofence =
            snap.geofence?.let {
                PolicySyncBuilder.GeofenceBounds(
                    latMin = it.latMin,
                    latMax = it.latMax,
                    lonMin = it.lonMin,
                    lonMax = it.lonMax,
                )
            }
        val app = context.wardriverApp()
        val gatewayMac = app.gatewayWifiRepository.gatewayInfoFlow.value?.gatewayMac
        val denyBssids =
            DenylistBssids.mergedManualPlusGateway(
                AppPreferences.parseLineList(snap.denylistBssidsText),
                gatewayMac,
            )
        val denySsids = AppPreferences.parseLineList(snap.denylistSsidsText)
        val ssidWatch = AppPreferences.parseLineList(snap.ssidWatchText)
        val legacyChannels =
            if (app.gatewayWifiRepository.supportsChannelPlanSync()) {
                emptyList()
            } else {
                snap.channelPlan.legacyFlatChannels()
            }
        return PolicySyncBuilder.buildWireBytes(
            roster = roster,
            geofence = geofence,
            denylistBssids = denyBssids,
            denylistSsids = denySsids,
            detectionDisableWifiMask = snap.detectionDisableWifiMask,
            detectionDisableBleMask = snap.detectionDisableBleMask,
            bleActiveScan = snap.bleActiveScan,
            ssidWatch = ssidWatch,
            scanAllowed = snap.scanAllowed,
            wardriveSuppressed = wardriveSuppressed,
            dwellMs = snap.channelPlan.dwellMs,
            channels = legacyChannels,
            seq = seq,
        )
    }

    /**
     * Build with PolicySync roster from Room (fully adopted + adopt-pending).
     * Assigns the next monotonic PackLink `seq` via [GatewayWifiRepository.nextPolicySyncSeq].
     */
    suspend fun buildWireBytesWithRoster(
        context: Context,
        wardriveSuppressed: Boolean = false,
    ): ByteArray? {
        val app = context.wardriverApp()
        val roster =
            app.database.fleetRosterDao().policyRoster().map { row ->
                PolicySyncBuilder.RosterEntry(
                    snifferId = row.snifferId,
                    mac = row.mac,
                    fleetSlot = row.fleetSlot,
                    roleWire = if (row.role == FleetRepository.ROLE_BLE) 1 else 0,
                )
            }
        val seq = app.gatewayWifiRepository.nextPolicySyncSeq()
        return buildWireBytes(context, roster, wardriveSuppressed, seq)
    }

    /** Push assembled policy to gateway when Wi-Fi link is up. */
    suspend fun pushIfConnected(
        context: Context,
        wardriveSuppressed: Boolean = false,
    ): PolicyPushResult {
        val app = context.wardriverApp()
        val connected =
            app.gatewayWifiRepository.connectionState.first() is
                GatewayWifiRepository.ConnectionState.Connected
        if (!connected) return PolicyPushResult.NotConnected
        return if (app.gatewayWifiRepository.supportsChannelPlanSync()) {
            ChannelPlanSyncAssembler.pushFullSyncIfConnected(context, wardriveSuppressed)
        } else {
            val policy =
                buildWireBytesWithRoster(context, wardriveSuppressed)
                    ?: return PolicyPushResult.PackFailed
            return app.gatewayWifiRepository.pushTimeSyncThenPolicy(policy)
        }
    }

    /** Push after roster or channel-plan change (alias for fleet CRUD). */
    suspend fun assembleAndPush(
        context: Context,
        wardriveSuppressed: Boolean = false,
    ): PolicyPushResult = pushIfConnected(context, wardriveSuppressed)
}
