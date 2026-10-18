package net.wardriver.mobile.data

import net.wardriver.mobile.data.GatewayWifiRepository.ConnectState
import net.wardriver.mobile.data.GatewayWifiRepository.ConnectionState
import net.wardriver.mobile.data.GatewayWifiRepository.GatewayInfo
import net.wardriver.mobile.data.GatewayWifiRepository.PendingTofu
import net.wardriver.mobile.wifi.PackLinkTcpSession
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.net.Socket

/**
 * First unit test for the extracted [GatewayLinkState]:
 * link state transitions (connect -> connected -> disconnected) and the
 * TOFU pending/claim flow.
 */
class GatewayLinkStateTest {

    private fun testGatewayInfo(capabilities: Int = 0): GatewayInfo =
        GatewayInfo(
            gatewayMac = "AA:BB:CC:DD:EE:FF",
            firmwareBuild = 1,
            packlinkVersion = 1,
            protoVersion = 6,
            spoolCapacityBytes = 0L,
            spoolUsedBytes = 0L,
            peerSlotsFree = 0,
            peerJoinWaiters = 0,
            fleetId = 0L,
            bootId = 0L,
            capabilities = capabilities,
        )

    private fun testSession(): PackLinkTcpSession =
        PackLinkTcpSession(ByteArrayInputStream(ByteArray(0)), ByteArrayOutputStream())

    @Test
    fun initialStateIsDisconnectedIdleWithEmptyFlows() {
        val link = GatewayLinkState()
        assertEquals(ConnectionState.Disconnected, link.connectionState.value)
        assertEquals(ConnectState.Idle, link.connectState.value)
        assertNull(link.pendingTofu.value)
        assertNull(link.gatewayInfo.value)
        assertNull(link.captureStatus.value)
        assertEquals(
            emptyList<FleetRepository.SnifferStatusRow>(),
            link.snifferStatus.value,
        )
        assertEquals(0L, link.observationBulkRxCount.value)
        assertNull(link.lastPolicySyncAck.value)
        assertNull(link.lastChannelPlanSyncAck.value)
        assertEquals(0, link.capabilitiesValue())
    }

    @Test
    fun connectConnectedThenDisconnectedTransition() {
        val link = GatewayLinkState()
        // connect
        link.setConnectionState(ConnectionState.Connecting)
        link.setConnectState(ConnectState.Joining)
        assertEquals(ConnectionState.Connecting, link.connectionState.value)
        assertEquals(ConnectState.Joining, link.connectState.value)
        // connected with gateway info carrying the channel-plan cap bit
        link.setConnectionState(ConnectionState.Connected)
        link.setConnectState(ConnectState.Idle)
        val info = testGatewayInfo()
        link.setGatewayInfo(info)
        assertEquals(ConnectionState.Connected, link.connectionState.value)
        assertEquals(ConnectState.Idle, link.connectState.value)
        assertSame(info, link.gatewayInfo.value)
        // reconnecting after link loss
        link.setConnectionState(ConnectionState.Reconnecting)
        assertEquals(ConnectionState.Reconnecting, link.connectionState.value)
        // disconnected resets to idle
        link.setConnectionState(ConnectionState.Disconnected)
        link.setConnectState(ConnectState.Idle)
        link.setGatewayInfo(null)
        assertEquals(ConnectionState.Disconnected, link.connectionState.value)
        assertEquals(ConnectState.Idle, link.connectState.value)
        assertNull(link.gatewayInfo.value)
    }

    @Test
    fun tofuPendingThenClaimedFlow() {
        val link = GatewayLinkState()
        val tofu = PendingTofu(Socket(), testSession(), generation = 7L)
        // gateway handshake left a TOFU pending for user confirmation
        link.setPendingTofu(tofu)
        link.setConnectState(ConnectState.AwaitingConfirmation)
        assertSame(tofu, link.pendingTofu.value)
        assertEquals(ConnectState.AwaitingConfirmation, link.connectState.value)
        // user confirms: claim clears the flow and hands back the same instance
        val claimed = link.claimPendingTofu()
        assertSame(tofu, claimed)
        assertNull(link.pendingTofu.value)
        // a second claim finds nothing pending
        assertNull(link.claimPendingTofu())
        // a cancel path (set null directly) also leaves nothing to claim
        link.setPendingTofu(tofu)
        link.setPendingTofu(null)
        assertNull(link.claimPendingTofu())
    }

    @Test
    fun capabilityBitReadsFromGatewayInfo() {
        val link = GatewayLinkState()
        val capChannelPlanSync = 1 shl 0
        val withCap = testGatewayInfo(capabilities = capChannelPlanSync)
        link.setGatewayInfo(withCap)
        assertEquals(capChannelPlanSync, link.capabilitiesValue())
        link.setGatewayInfo(null)
        assertEquals(0, link.capabilitiesValue())
        assertFalse(link.capabilitiesValue() and capChannelPlanSync != 0)
    }

    @Test
    fun observationBulkRxCountIncrements() {
        val link = GatewayLinkState()
        link.incrementObservationBulkRxCount()
        link.incrementObservationBulkRxCount()
        assertEquals(2L, link.observationBulkRxCount.value)
        link.setObservationBulkRxCount(0L)
        assertEquals(0L, link.observationBulkRxCount.value)
    }
}
