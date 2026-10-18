package net.wardriver.mobile.data

import android.content.Context
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.policy.ChannelPlanSyncBuilder
import net.wardriver.mobile.wardriverApp

/**
 * Merges [AppPreferences] channel plan and fleet manual band/pin rows into
 * PackLink `ChannelPlanSyncV1` wire bytes.
 */
object ChannelPlanSyncAssembler {
    suspend fun buildWireBytes(
        context: Context,
        seq: Int = 0,
    ): ByteArray? {
        val app = context.wardriverApp()
        val snap = app.appPreferences.snapshot()
        val plan = snap.channelPlan
        val sniffers24 = mutableListOf<Int>()
        val sniffers5 = mutableListOf<Int>()
        val pins = mutableListOf<ChannelPlanSyncBuilder.PinEntry>()
        if (plan.isManualBandMode()) {
            val roster = app.database.fleetRosterDao().adopted()
            for (row in roster) {
                if (row.role != FleetRepository.ROLE_WIFI) continue
                when (row.band) {
                    "5" -> sniffers5.add(row.snifferId)
                    else -> sniffers24.add(row.snifferId)
                }
                row.pinnedChannel?.let { ch ->
                    pins.add(ChannelPlanSyncBuilder.PinEntry(row.snifferId, ch))
                }
            }
        }
        return ChannelPlanSyncBuilder.buildWireBytes(
            revision = plan.revision,
            dwellMs = plan.dwellMs,
            include24Ghz = plan.include24Ghz,
            include5Ghz = plan.include5Ghz,
            includeDfs = plan.includeDfs,
            bandMode = plan.bandMode,
            channels24 = plan.channels24,
            channels5 = plan.channels5,
            channels5Dfs = plan.channels5Dfs,
            sniffers24 = sniffers24,
            sniffers5 = sniffers5,
            pins = pins,
            seq = seq,
        )
    }

    suspend fun buildWireBytesWithSeq(context: Context): ByteArray? {
        val seq = context.wardriverApp().gatewayWifiRepository.nextChannelPlanSyncSeq()
        return buildWireBytes(context, seq)
    }

    /** Push channel plan when gateway supports ChannelPlanSync; else legacy PolicySync channels. */
    suspend fun assembleAndPush(context: Context) {
        val app = context.wardriverApp()
        val connected =
            app.gatewayWifiRepository.connectionState.first() is
                GatewayWifiRepository.ConnectionState.Connected
        if (!connected) return
        if (app.gatewayWifiRepository.supportsChannelPlanSync()) {
            buildWireBytesWithSeq(context)?.let { bytes ->
                app.gatewayWifiRepository.writeChannelPlanSync(bytes)
            }
        } else {
            PolicySyncAssembler.pushIfConnected(context)
        }
    }

    /** TimeSync + PolicySync + ChannelPlanSync after connect or full resync. */
    suspend fun pushFullSyncIfConnected(
        context: Context,
        wardriveSuppressed: Boolean = false,
    ): PolicyPushResult {
        val app = context.wardriverApp()
        val connected =
            app.gatewayWifiRepository.connectionState.first() is
                GatewayWifiRepository.ConnectionState.Connected
        if (!connected) return PolicyPushResult.NotConnected
        val policy =
            PolicySyncAssembler.buildWireBytesWithRoster(context, wardriveSuppressed)
                ?: return PolicyPushResult.PackFailed
        return if (app.gatewayWifiRepository.supportsChannelPlanSync()) {
            val channelPlan = buildWireBytesWithSeq(context) ?: return PolicyPushResult.PackFailed
            app.gatewayWifiRepository.pushTimeSyncThenPolicyThenChannelPlan(policy, channelPlan)
        } else {
            app.gatewayWifiRepository.pushTimeSyncThenPolicy(policy)
        }
    }
}
