package net.wardriver.mobile.data

import android.content.Context
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.min
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.ingest.CatchUpIngestManager
import net.wardriver.mobile.ingest.EventNotifyHandler
import net.wardriver.mobile.ingest.ObservationIngestor
import net.wardriver.mobile.ingest.SpoolReplayAssembler
import net.wardriver.mobile.ingest.TimeSyncScheduler
import net.wardriver.mobile.service.GatewayLinkService
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.ui.WifiPermissions
import net.wardriver.mobile.wifi.GatewayTrustStore
import net.wardriver.mobile.wifi.PackLinkTcpSession
import net.wardriver.mobile.wifi.PackLinkWifiManager
import net.wardriver.mobile.wardriverApp
import java.net.Socket
import java.util.concurrent.CopyOnWriteArraySet

/**
 * Gateway SoftAP PackLink link; independent of wardrive session lifecycle.
 *
 * Thin coordinator over [PackLinkTcpSession]: link state flows live in
 * [GatewayLinkState], spool-replay assembly in [SpoolReplayAssembler], and
 * catch-up ingest in [CatchUpIngestManager].
 */
class GatewayWifiRepository(
    context: Context,
    private val appPreferences: AppPreferences,
) {
    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val frameListeners = CopyOnWriteArraySet<(ByteArray) -> Unit>()
    private val wifiManager = PackLinkWifiManager(appContext)
    private val trustStore = GatewayTrustStore(appContext)
    private val linkState = GatewayLinkState()
    private val eventNotifyHandler =
        EventNotifyHandler(scope, appContext, appContext.wardriverApp().database)
    private val timeSyncScheduler = TimeSyncScheduler(scope, ::writeTimeSync)
    private val spoolReplayAssembler =
        SpoolReplayAssembler(
            onInnerFrame = { inner -> dispatchReplayFrame(inner, currentActiveSessionGeneration()) },
            onReplayComplete = { nonce -> sendSpoolReplayConfirmation(nonce) },
        )
    private val catchUpIngestManager =
        CatchUpIngestManager(
            db = appContext.wardriverApp().database,
            scope = scope,
            hooks =
                object : CatchUpIngestManager.Hooks {
                    override fun addFrameListener(listener: (ByteArray) -> Unit) {
                        this@GatewayWifiRepository.addFrameListener(listener)
                    }

                    override fun removeFrameListener(listener: (ByteArray) -> Unit) {
                        this@GatewayWifiRepository.removeFrameListener(listener)
                    }

                    override fun addReplayFrameListener(listener: suspend (ByteArray) -> Unit) {
                        spoolReplayAssembler.addReplayFrameListener(listener)
                    }

                    override fun removeReplayFrameListener(listener: suspend (ByteArray) -> Unit) {
                        spoolReplayAssembler.removeReplayFrameListener(listener)
                    }

                    override fun setEventNotifySessionId(sessionId: Long) {
                        this@GatewayWifiRepository.setEventNotifySessionId(sessionId)
                    }

                    override fun currentGatewayMac(): String? =
                        linkState.gatewayInfoValue()?.gatewayMac

                    override fun isWardriveActive(): Boolean =
                        WardriveSessionService.sessionIdFlow.value != null
                },
        )

    private var readJob: Job? = null
    private var socket: Socket? = null
    private var tcpSession: PackLinkTcpSession? = null
    private val writerLock = Any()
    private val pendingWrites = mutableListOf<PendingWrite>()
    private val writerSignal = Channel<Unit>(Channel.CONFLATED)
    private var connectGeneration = 0L
    private var activeSessionGeneration = 0L
    private var reconnectJob: Job? = null

    @Volatile
    private var userWantsConnected = false

    // ── read-only link StateFlows (delegate to GatewayLinkState) ────────────
    val connectionState: StateFlow<ConnectionState> = linkState.connectionState
    val gatewayInfoFlow: StateFlow<GatewayInfo?> = linkState.gatewayInfo
    val connectState: StateFlow<ConnectState> = linkState.connectState
    val pendingTofu: StateFlow<PendingTofu?> = linkState.pendingTofu
    val captureStatusFlow: StateFlow<CaptureStatus?> = linkState.captureStatus
    val snifferStatusFlow: StateFlow<List<FleetRepository.SnifferStatusRow>> =
        linkState.snifferStatus
    val observationBulkRxCount: StateFlow<Long> = linkState.observationBulkRxCount
    val lastPolicySyncAck: StateFlow<NativeCore.PolicySyncAck?> = linkState.lastPolicySyncAck
    val lastChannelPlanSyncAck: StateFlow<NativeCore.ChannelPlanSyncAck?> =
        linkState.lastChannelPlanSyncAck

    @Volatile
    private var policySyncSeq: Long = 0L

    @Volatile
    private var channelPlanSyncSeq: Long = 0L

    @Volatile
    private var expectedPolicyAckSeq: Long? = null

    @Volatile
    private var expectedPolicyAckGeneration: Long = 0L

    @Volatile
    private var expectedChannelPlanAckSeq: Long? = null

    @Volatile
    private var expectedChannelPlanAckGeneration: Long = 0L

    @Volatile
    private var latestRequestedChannelPlanSeq: Long? = null

    @Volatile
    private var latestRequestedPolicySeq: Long? = null

    @Volatile
    private var statusAnchorGatewayMs: Long = 0L

    @Volatile
    private var statusAnchorWallMs: Long = 0L

    init {
        scope.launch { writerLoop() }
    }

    /** Next monotonic PackLink frame seq for uplink PolicySync (PolicySyncAck correlation). */
    @Synchronized
    fun nextPolicySyncSeq(): Int {
        policySyncSeq = (policySyncSeq + 1L) and 0xFFFF_FFFFL
        if (policySyncSeq == 0L) policySyncSeq = 1L
        return policySyncSeq.toInt()
    }

    /** Next monotonic PackLink frame seq for uplink ChannelPlanSync (ack correlation). */
    @Synchronized
    fun nextChannelPlanSyncSeq(): Int {
        channelPlanSyncSeq = (channelPlanSyncSeq + 1L) and 0xFFFF_FFFFL
        if (channelPlanSyncSeq == 0L) channelPlanSyncSeq = 1L
        return channelPlanSyncSeq.toInt()
    }

    /** True when gateway `GatewayInfo` advertises dedicated ChannelPlanSync (cap bit 0). */
    fun supportsChannelPlanSync(): Boolean =
        linkState.capabilitiesValue() and CAP_CHANNEL_PLAN_SYNC != 0

    /**
     * Gateway mono clock extrapolated between anchors.
     * Anchors come from TimeSync replies and SnifferStatus `gateway_mono_ms` (not max last_seen).
     */
    fun extrapolatedGatewayNowMs(): Long {
        if (statusAnchorGatewayMs <= 0L || statusAnchorWallMs <= 0L) return 0L
        return statusAnchorGatewayMs + (System.currentTimeMillis() - statusAnchorWallMs)
    }

    private fun noteGatewayMono(gatewayMonoMs: Long) {
        if (gatewayMonoMs <= 0L) return
        statusAnchorGatewayMs = gatewayMonoMs
        statusAnchorWallMs = System.currentTimeMillis()
    }

    fun canConnect(): Boolean = WifiPermissions.canJoinSoftAp(appContext)

    fun hasTrustedGateway(): Boolean = trustStore.hasTrustedGateway()

    /** Active wardrive session id for `EventNotify` detection rows (0 when idle). */
    fun setEventNotifySessionId(sessionId: Long) {
        eventNotifyHandler.sessionId = sessionId
    }

    fun addFrameListener(listener: (ByteArray) -> Unit) {
        frameListeners.add(listener)
    }

    fun removeFrameListener(listener: (ByteArray) -> Unit) {
        frameListeners.remove(listener)
    }

    fun addReplayFrameListener(listener: suspend (ByteArray) -> Unit) {
        spoolReplayAssembler.addReplayFrameListener(listener)
    }

    fun removeReplayFrameListener(listener: suspend (ByteArray) -> Unit) {
        spoolReplayAssembler.removeReplayFrameListener(listener)
    }

    fun connect() {
        userWantsConnected = true
        val generation =
            synchronized(writerLock) {
                val current = linkState.connectionStateValue()
                if (current is ConnectionState.Connected ||
                    current is ConnectionState.Connecting ||
                    current is ConnectionState.Reconnecting
                ) {
                    return
                }
                connectGeneration += 1
                connectGeneration
            }
        if (!canConnect()) {
            userWantsConnected = false
            linkState.setConnectState(ConnectState.Failed("Location / Wi-Fi permission not granted"))
            linkState.setConnectionState(ConnectionState.Error("Location / Wi-Fi permission not granted"))
            return
        }
        launchJoin(generation, isReconnect = false)
    }

    private fun launchJoin(
        generation: Long,
        isReconnect: Boolean,
    ) {
        scope.launch {
            try {
                if (!isCurrentConnectGeneration(generation)) return@launch
                linkState.setConnectState(ConnectState.Joining)
                linkState.setConnectionState(
                    if (isReconnect) {
                        ConnectionState.Reconnecting
                    } else {
                        ConnectionState.Connecting
                    },
                )
                val creds = appPreferences.gatewaySoftApCreds()
                if (isReconnect) {
                    wifiManager.reconnectToSoftAp(
                        ssid = creds.ssid,
                        password = creds.password,
                        onReady = { sock -> scope.launch { runHandshake(sock, generation) } },
                        onFailed = { msg -> handleJoinFailed(generation, msg, isReconnect) },
                        onLinkLost = { onWifiLinkLost() },
                    )
                } else {
                    wifiManager.connectToSoftAp(
                        ssid = creds.ssid,
                        password = creds.password,
                        onReady = { sock -> scope.launch { runHandshake(sock, generation) } },
                        onFailed = { msg -> handleJoinFailed(generation, msg, isReconnect) },
                        onLinkLost = { onWifiLinkLost() },
                    )
                }
            } catch (e: Exception) {
                handleJoinFailed(generation, e.message ?: "connect failed", isReconnect)
            }
        }
    }

    private fun handleJoinFailed(
        generation: Long,
        message: String,
        isReconnect: Boolean,
    ) {
        if (!isCurrentConnectGeneration(generation)) return
        if (userWantsConnected && isReconnect) {
            linkState.setConnectState(ConnectState.Joining)
            linkState.setConnectionState(ConnectionState.Reconnecting)
            return
        }
        linkState.setConnectState(ConnectState.Failed(message))
        linkState.setConnectionState(ConnectionState.Error(message))
    }

    private fun onWifiLinkLost() {
        val (generation, sessionSocket) = currentLinkLossContext()
        scope.launch { handleLinkLoss(generation, sessionSocket) }
    }

    private fun currentLinkLossContext(): Pair<Long?, Socket?> =
        synchronized(writerLock) {
            if (activeSessionGeneration != 0L) {
                activeSessionGeneration to socket
            } else {
                connectGeneration to null
            }
        }

    private fun handleLinkLoss(
        expectedGeneration: Long?,
        expectedSocket: Socket?,
    ) {
        teardownTransport(expectedGeneration, expectedSocket)
        if (userWantsConnected) {
            scheduleReconnect()
        }
    }

    private fun scheduleReconnect() {
        if (!userWantsConnected) return
        reconnectJob?.cancel()
        reconnectJob =
            scope.launch {
                var backoffMs = RECONNECT_INITIAL_MS
                while (isActive && userWantsConnected) {
                    linkState.setConnectionState(ConnectionState.Reconnecting)
                    delay(backoffMs)
                    backoffMs = min(backoffMs * 2, RECONNECT_MAX_MS)
                    val generation =
                        synchronized(writerLock) {
                            connectGeneration
                        }
                    if (!userWantsConnected || !isCurrentConnectGeneration(generation)) return@launch
                    if (linkState.connectionStateValue() is ConnectionState.Connected) return@launch
                    launchJoin(generation, isReconnect = true)
                    val deadline = System.currentTimeMillis() + RECONNECT_ATTEMPT_TIMEOUT_MS
                    while (isActive && userWantsConnected && System.currentTimeMillis() < deadline) {
                        when (linkState.connectionStateValue()) {
                            is ConnectionState.Connected -> return@launch
                            is ConnectionState.Error -> break
                            else -> delay(500)
                        }
                    }
                }
            }
    }

    fun confirmPendingTofu() {
        val pending =
            synchronized(writerLock) {
                linkState.claimPendingTofu() ?: return
            }
        scope.launch {
            try {
                if (!isCurrentConnectGeneration(pending.generation)) {
                    pending.socket.close()
                    return@launch
                }
                trustStore.markTrusted()
                finishConnect(pending.socket, pending.session, pending.generation)
            } catch (e: Exception) {
                pending.socket.close()
                if (isCurrentConnectGeneration(pending.generation)) {
                    linkState.setConnectState(ConnectState.Failed(e.message ?: "TOFU failed"))
                    linkState.setConnectionState(ConnectionState.Error(e.message ?: "TOFU failed"))
                }
            }
        }
    }

    fun cancelPendingTofu() {
        linkState.pendingTofuValue()?.socket?.close()
        linkState.setPendingTofu(null)
        fullDisconnect()
    }

    fun connectPreferred() = connect()

    fun forgetGateway() {
        scope.launch {
            try {
                writeFrameAndAwait(PackLinkTcpSession.packTrustForget())
            } catch (_: Exception) {
            }
            trustStore.forgetGateway()
            disconnect()
        }
    }

    fun disconnect() {
        fullDisconnect()
    }

    private fun teardownTransport(
        expectedGeneration: Long?,
        expectedSocket: Socket?,
    ) {
        val (jobToCancel, socketToClose) =
            synchronized(writerLock) {
                if (expectedGeneration != null) {
                    if (expectedSocket != null) {
                        if (activeSessionGeneration != expectedGeneration || socket !== expectedSocket) {
                            return
                        }
                    } else if (activeSessionGeneration != 0L &&
                        activeSessionGeneration != expectedGeneration
                    ) {
                        return
                    }
                }
                activeSessionGeneration = 0L
                val oldJob = readJob
                val oldSocket = socket
                readJob = null
                socket = null
                tcpSession = null
                expectedPolicyAckSeq = null
                expectedPolicyAckGeneration = 0L
                expectedChannelPlanAckSeq = null
                expectedChannelPlanAckGeneration = 0L
                latestRequestedPolicySeq = null
                latestRequestedChannelPlanSeq = null
                pendingWrites.forEach {
                    it.completion?.completeExceptionally(
                        IllegalStateException("gateway link lost"),
                    )
                }
                pendingWrites.clear()
                oldJob to oldSocket
            }
        jobToCancel?.cancel()
        timeSyncScheduler.stop()
        spoolReplayAssembler.reset()
        try {
            socketToClose?.close()
        } catch (_: Exception) {
        }
    }

    private fun fullDisconnect() {
        userWantsConnected = false
        reconnectJob?.cancel()
        reconnectJob = null
        val (jobToCancel, socketToClose) =
            synchronized(writerLock) {
                connectGeneration += 1
                activeSessionGeneration = 0L
                val oldJob = readJob
                val oldSocket = socket
                readJob = null
                socket = null
                tcpSession = null
                expectedPolicyAckSeq = null
                expectedPolicyAckGeneration = 0L
                expectedChannelPlanAckSeq = null
                expectedChannelPlanAckGeneration = 0L
                latestRequestedPolicySeq = null
                latestRequestedChannelPlanSeq = null
                pendingWrites.forEach {
                    it.completion?.completeExceptionally(
                        IllegalStateException("gateway disconnected"),
                    )
                }
                pendingWrites.clear()
                oldJob to oldSocket
            }
        jobToCancel?.cancel()
        timeSyncScheduler.stop()
        spoolReplayAssembler.reset()
        try {
            socketToClose?.close()
        } catch (_: Exception) {
        }
        wifiManager.release()
        catchUpIngestManager.tearDown()
        GatewayLinkService.stop(appContext)
        linkState.setConnectionState(ConnectionState.Disconnected)
        linkState.setGatewayInfo(null)
        linkState.setConnectState(ConnectState.Idle)
        linkState.setSnifferStatus(emptyList())
        statusAnchorGatewayMs = 0L
        statusAnchorWallMs = 0L
        linkState.setObservationBulkRxCount(0L)
        linkState.setLastPolicySyncAck(null)
        linkState.setLastChannelPlanSyncAck(null)
    }

    /** Queue a PackLink ChannelPlanSync frame write (coalesced like PolicySync). */
    fun writeChannelPlanSync(payload: ByteArray) {
        linkState.setLastChannelPlanSyncAck(null)
        enqueueWrite(payload, isChannelPlanSync = true)
    }

    fun writeTimeSync(payload: ByteArray) {
        enqueueWrite(payload)
    }

    /** Uplink TimeSync when gateway link is connected (GPS fix / heartbeat). */
    fun sendTimeSyncIfConnected() {
        if (linkState.connectionStateValue() !is ConnectionState.Connected) return
        timeSyncScheduler.sendNow()
    }

    /** TimeSync then PolicySync in order (wardrive session start / reconnect). */
    suspend fun pushTimeSyncThenPolicy(policyBytes: ByteArray): PolicyPushResult {
        return withContext(Dispatchers.IO) {
            if (linkState.connectionStateValue() !is ConnectionState.Connected) {
                return@withContext PolicyPushResult.NotConnected
            }
            val timeSync = timeSyncScheduler.buildWireBytes()
            linkState.setLastPolicySyncAck(null)
            val completions = mutableListOf<CompletableDeferred<Unit>>()
            synchronized(writerLock) {
                val generation = activeSessionGeneration
                if (generation == 0L || tcpSession == null) {
                    return@withContext PolicyPushResult.NotConnected
                }
                expectedPolicyAckSeq = null
                expectedPolicyAckGeneration = 0L
                latestRequestedPolicySeq = packLinkSeq(policyBytes)
                pendingWrites.removeAll {
                    if (it.isPolicySync) {
                        it.completion?.completeExceptionally(
                            IllegalStateException("PolicySync superseded"),
                        )
                        true
                    } else {
                        false
                    }
                }
                if (timeSync != null) {
                    val tsDone = CompletableDeferred<Unit>()
                    completions.add(tsDone)
                    pendingWrites.add(
                        PendingWrite(
                            timeSync,
                            generation = generation,
                            completion = tsDone,
                        ),
                    )
                }
                val policyDone = CompletableDeferred<Unit>()
                completions.add(policyDone)
                pendingWrites.add(
                    PendingWrite(
                        policyBytes,
                        isPolicySync = true,
                        generation = generation,
                        completion = policyDone,
                    ),
                )
            }
            writerSignal.trySend(Unit)
            awaitWriteCompletions(completions)
        }
    }

    /** TimeSync, PolicySync, then optional ChannelPlanSync (connect / full resync). */
    suspend fun pushTimeSyncThenPolicyThenChannelPlan(
        policyBytes: ByteArray,
        channelPlanBytes: ByteArray?,
    ): PolicyPushResult {
        return withContext(Dispatchers.IO) {
            if (linkState.connectionStateValue() !is ConnectionState.Connected) {
                return@withContext PolicyPushResult.NotConnected
            }
            val timeSync = timeSyncScheduler.buildWireBytes()
            linkState.setLastPolicySyncAck(null)
            linkState.setLastChannelPlanSyncAck(null)
            val completions = mutableListOf<CompletableDeferred<Unit>>()
            synchronized(writerLock) {
                val generation = activeSessionGeneration
                if (generation == 0L || tcpSession == null) {
                    return@withContext PolicyPushResult.NotConnected
                }
                expectedPolicyAckSeq = null
                expectedPolicyAckGeneration = 0L
                expectedChannelPlanAckSeq = null
                expectedChannelPlanAckGeneration = 0L
                latestRequestedPolicySeq = packLinkSeq(policyBytes)
                latestRequestedChannelPlanSeq =
                    channelPlanBytes?.let { packLinkSeq(it) }
                pendingWrites.removeAll {
                    if (it.isPolicySync || it.isChannelPlanSync) {
                        it.completion?.completeExceptionally(
                            IllegalStateException("sync superseded"),
                        )
                        true
                    } else {
                        false
                    }
                }
                if (timeSync != null) {
                    val tsDone = CompletableDeferred<Unit>()
                    completions.add(tsDone)
                    pendingWrites.add(PendingWrite(timeSync, generation = generation, completion = tsDone))
                }
                val policyDone = CompletableDeferred<Unit>()
                completions.add(policyDone)
                pendingWrites.add(
                    PendingWrite(
                        policyBytes,
                        isPolicySync = true,
                        generation = generation,
                        completion = policyDone,
                    ),
                )
                channelPlanBytes?.let { planBytes ->
                    val planDone = CompletableDeferred<Unit>()
                    completions.add(planDone)
                    pendingWrites.add(
                        PendingWrite(
                            planBytes,
                            isChannelPlanSync = true,
                            generation = generation,
                            completion = planDone,
                        ),
                    )
                }
            }
            writerSignal.trySend(Unit)
            awaitWriteCompletions(completions)
        }
    }

    private suspend fun awaitWriteCompletions(
        completions: List<CompletableDeferred<Unit>>,
    ): PolicyPushResult {
        for (deferred in completions) {
            try {
                deferred.await()
            } catch (e: IllegalStateException) {
                val msg = e.message.orEmpty()
                return when {
                    msg.contains("superseded", ignoreCase = true) -> PolicyPushResult.Superseded
                    msg.contains("stale", ignoreCase = true) -> PolicyPushResult.StaleSession
                    msg.contains("not connected", ignoreCase = true) -> PolicyPushResult.NotConnected
                    else -> PolicyPushResult.WriteFailed(msg.ifBlank { "gateway write failed" })
                }
            } catch (e: Exception) {
                return PolicyPushResult.WriteFailed(e.message ?: "gateway write failed")
            }
        }
        return PolicyPushResult.Pushed
    }

    fun writeCaptureCmd(payload: ByteArray) {
        enqueueWrite(payload)
    }

    suspend fun pushSoftApConfig(ssid: String, password: String) {
        appPreferences.saveGatewaySoftApCreds(ssid, password)
        val wire = PackLinkTcpSession.packSoftApConfig(ssid, password, seq = System.currentTimeMillis().toInt())
        enqueueWrite(wire)
    }

    fun startWifiProbeCapture(durationSec: Int = 60) {
        val wire =
            NativeCore.packCaptureCmd(
                jobId = 1,
                durationSec = durationSec,
                seq = System.currentTimeMillis().toInt(),
            ) ?: return
        writeCaptureCmd(wire)
    }

    suspend fun applyStoredGeofencePolicy() {
        ChannelPlanSyncAssembler.pushFullSyncIfConnected(appContext)
    }

    private fun enqueueWrite(
        payload: ByteArray,
        isPolicySync: Boolean = false,
        isChannelPlanSync: Boolean = false,
        completion: CompletableDeferred<Unit>? = null,
    ) {
        synchronized(writerLock) {
            val generation = activeSessionGeneration
            if (generation == 0L || tcpSession == null) {
                completion?.completeExceptionally(IllegalStateException("gateway not connected"))
                return
            }
            if (isPolicySync) {
                linkState.setLastPolicySyncAck(null)
                expectedPolicyAckSeq = null
                expectedPolicyAckGeneration = 0L
                latestRequestedPolicySeq = packLinkSeq(payload)
                pendingWrites.removeAll {
                    if (it.isPolicySync) {
                        it.completion?.completeExceptionally(
                            IllegalStateException("PolicySync superseded"),
                        )
                        true
                    } else {
                        false
                    }
                }
            }
            if (isChannelPlanSync) {
                linkState.setLastChannelPlanSyncAck(null)
                expectedChannelPlanAckSeq = null
                expectedChannelPlanAckGeneration = 0L
                latestRequestedChannelPlanSeq = packLinkSeq(payload)
                pendingWrites.removeAll {
                    if (it.isChannelPlanSync) {
                        it.completion?.completeExceptionally(
                            IllegalStateException("ChannelPlanSync superseded"),
                        )
                        true
                    } else {
                        false
                    }
                }
            }
            pendingWrites.add(
                PendingWrite(
                    payload,
                    isPolicySync,
                    isChannelPlanSync,
                    generation,
                    completion,
                ),
            )
        }
        writerSignal.trySend(Unit)
    }

    private suspend fun writeFrameAndAwait(payload: ByteArray) {
        val completion = CompletableDeferred<Unit>()
        enqueueWrite(payload, completion = completion)
        completion.await()
    }

    private fun sendSpoolReplayConfirmation(snapshotNonce: Long) {
        val wire = timeSyncScheduler.buildReplayConfirmationWire(snapshotNonce) ?: return
        enqueueWrite(wire)
    }

    private suspend fun writerLoop() {
        while (true) {
            writerSignal.receive()
            while (true) {
                val pending =
                    synchronized(writerLock) {
                        if (pendingWrites.isEmpty()) null else pendingWrites.removeAt(0)
                    } ?: break
                val emittedPolicySeq =
                    if (pending.isPolicySync) packLinkSeq(pending.payload) else null
                val emittedChannelPlanSeq =
                    if (pending.isChannelPlanSync) packLinkSeq(pending.payload) else null
                val session =
                    synchronized(writerLock) {
                        if (activeSessionGeneration != pending.generation) {
                            null
                        } else {
                            if (pending.isPolicySync &&
                                latestRequestedPolicySeq == emittedPolicySeq
                            ) {
                                expectedPolicyAckSeq = emittedPolicySeq
                                expectedPolicyAckGeneration = pending.generation
                            }
                            if (pending.isChannelPlanSync &&
                                latestRequestedChannelPlanSeq == emittedChannelPlanSeq
                            ) {
                                expectedChannelPlanAckSeq = emittedChannelPlanSeq
                                expectedChannelPlanAckGeneration = pending.generation
                            }
                            tcpSession
                        }
                    }
                if (session == null) {
                    pending.completion?.completeExceptionally(
                        IllegalStateException("stale gateway session"),
                    )
                    continue
                }
                try {
                    session.writeFrame(pending.payload)
                    pending.completion?.complete(Unit)
                } catch (_: Exception) {
                    if (pending.isPolicySync) {
                        synchronized(writerLock) {
                            if (activeSessionGeneration == pending.generation &&
                                expectedPolicyAckSeq == emittedPolicySeq
                            ) {
                                expectedPolicyAckSeq = null
                                expectedPolicyAckGeneration = 0L
                            }
                        }
                    }
                    if (pending.isChannelPlanSync) {
                        synchronized(writerLock) {
                            if (activeSessionGeneration == pending.generation &&
                                expectedChannelPlanAckSeq == emittedChannelPlanSeq
                            ) {
                                expectedChannelPlanAckSeq = null
                                expectedChannelPlanAckGeneration = 0L
                            }
                        }
                    }
                    pending.completion?.completeExceptionally(
                        IllegalStateException("gateway write failed"),
                    )
                    // Dead socket: read loop disconnects or caller retries on next connect.
                }
            }
        }
    }

    private fun packLinkSeq(frame: ByteArray): Long? {
        if (frame.size < 14) return null
        return readU32Le(frame, 10)
    }

    private fun isCurrentConnectGeneration(generation: Long): Boolean =
        synchronized(writerLock) {
            connectGeneration == generation
        }

    private fun isActiveSession(
        generation: Long,
        sessionSocket: Socket,
    ): Boolean =
        synchronized(writerLock) {
            activeSessionGeneration == generation && socket === sessionSocket
        }

    private fun currentActiveSessionGeneration(): Long =
        synchronized(writerLock) {
            activeSessionGeneration
        }

    private suspend fun runHandshake(sock: Socket, generation: Long) {
        try {
            if (!isCurrentConnectGeneration(generation)) {
                sock.close()
                return
            }
            val session = PackLinkTcpSession(sock.getInputStream(), sock.getOutputStream())
            val phoneId = trustStore.phoneId()
            session.writeFrame(PackLinkTcpSession.packPhoneHello(phoneId))
            val reply = session.readFrame()
            if (!isCurrentConnectGeneration(generation)) {
                sock.close()
                return
            }
            val parsed = NativeCore.unpackPackLinkFrame(reply)
            if (parsed !is NativeCore.PackLinkFrameResult.Ok) {
                val detail =
                    (parsed as? NativeCore.PackLinkFrameResult.Error)?.message ?: "unpack failed"
                throw IllegalStateException("handshake parse failed: $detail")
            }
            when (parsed.msgType) {
                MSG_TYPE_TRUST_OK -> {
                    if (!trustStore.hasTrustedGateway()) {
                        try {
                            val infoFrame = session.readFrame()
                            if (!isCurrentConnectGeneration(generation)) {
                                sock.close()
                                return
                            }
                            handleGatewayInfoBytes(infoFrame)
                        } catch (_: Exception) {
                        }
                        if (!isCurrentConnectGeneration(generation)) {
                            sock.close()
                            return
                        }
                        linkState.setPendingTofu(PendingTofu(sock, session, generation))
                        linkState.setConnectState(ConnectState.AwaitingConfirmation)
                        return
                    }
                    try {
                        val infoFrame = session.readFrame()
                        if (!isCurrentConnectGeneration(generation)) {
                            sock.close()
                            return
                        }
                        handleGatewayInfoBytes(infoFrame)
                    } catch (_: Exception) {
                    }
                    finishConnect(sock, session, generation)
                }
                MSG_TYPE_TRUST_REJECT -> {
                    throw IllegalStateException("Gateway rejected this phone (TOFU mismatch)")
                }
                else -> throw IllegalStateException("expected TrustOk, got ${parsed.msgTypeName}")
            }
        } catch (e: Exception) {
            sock.close()
            if (isCurrentConnectGeneration(generation)) {
                if (userWantsConnected &&
                    (linkState.connectionStateValue() is ConnectionState.Reconnecting ||
                        linkState.connectionStateValue() is ConnectionState.Connecting)
                ) {
                    linkState.setConnectState(ConnectState.Joining)
                    linkState.setConnectionState(ConnectionState.Reconnecting)
                    scheduleReconnect()
                } else {
                    linkState.setConnectState(ConnectState.Failed(e.message ?: "handshake failed"))
                    linkState.setConnectionState(ConnectionState.Error(e.message ?: "handshake failed"))
                }
            }
        }
    }

    private suspend fun finishConnect(
        sock: Socket,
        session: PackLinkTcpSession,
        generation: Long,
    ) {
        val accepted =
            synchronized(writerLock) {
                if (connectGeneration != generation || activeSessionGeneration != 0L) {
                    false
                } else {
                    socket = sock
                    tcpSession = session
                    activeSessionGeneration = generation
                    true
                }
            }
        if (!accepted) {
            sock.close()
            return
        }
        reconnectJob?.cancel()
        reconnectJob = null
        linkState.setConnectState(ConnectState.Idle)
        linkState.setConnectionState(ConnectionState.Connected)
        GatewayLinkService.ensureRunning(appContext)
        catchUpIngestManager.ensureStarted()
        scope.launch { ChannelPlanSyncAssembler.pushFullSyncIfConnected(appContext) }
        timeSyncScheduler.startHeartbeat()
        startReadLoop(sock, session, generation)
    }

    /** Promote catch-up session + ingestor when wardrive starts on an active gateway link. */
    suspend fun promoteCatchUpForWardrive(): Pair<Long, ObservationIngestor>? =
        catchUpIngestManager.promoteForWardriveWithIngestor()

    /** Close catch-up ingest before an upload run; returns whether catch-up was active. */
    suspend fun prepareCatchUpForUpload(): Boolean = catchUpIngestManager.tearDownForUpload()

    /** Restart catch-up after upload when wardrive remains idle. */
    fun resumeCatchUpAfterUploadIfNeeded() {
        catchUpIngestManager.resumeAfterUploadIfNeeded()
    }

    private fun startReadLoop(
        sessionSocket: Socket,
        session: PackLinkTcpSession,
        generation: Long,
    ) {
        val previousJob =
            synchronized(writerLock) {
                val old = readJob
                readJob =
                    scope.launch {
                        while (isActive && isActiveSession(generation, sessionSocket)) {
                            try {
                                val frame = session.readFrame()
                                if (!isActiveSession(generation, sessionSocket)) break
                                dispatchFrame(frame, generation)
                            } catch (_: Exception) {
                                break
                            }
                        }
                        handleLinkLoss(generation, sessionSocket)
                    }
                old
            }
        previousJob?.cancel()
    }

    private suspend fun dispatchFrame(
        frame: ByteArray,
        sessionGeneration: Long,
    ) {
        when (val parsed = NativeCore.unpackPackLinkFrame(frame)) {
            is NativeCore.PackLinkFrameResult.Ok -> {
                when (parsed.msgType) {
                    MSG_TYPE_GATEWAY_INFO -> handleGatewayInfoBytes(frame)
                    MSG_TYPE_EVENT_NOTIFY -> eventNotifyHandler.onEventNotifyFrame(frame)
                    MSG_TYPE_SPOOL_REPLAY -> spoolReplayAssembler.onSpoolReplayFrame(frame)
                    MSG_TYPE_TIME_SYNC -> handleTimeSyncBytes(frame)
                    MSG_TYPE_SNIFFER_STATUS -> handleSnifferStatusBytes(frame)
                    MSG_TYPE_CAPTURE_STATUS -> handleCaptureStatusBytes(frame)
                    MSG_TYPE_POLICY_SYNC_ACK -> handlePolicySyncAckBytes(frame, sessionGeneration)
                    MSG_TYPE_CHANNEL_PLAN_SYNC_ACK ->
                        handleChannelPlanSyncAckBytes(frame, sessionGeneration)
                    MSG_TYPE_FLEET_UNBIND_NOTIFY -> handleFleetUnbindNotify()
                    MSG_TYPE_OBSERVATION_BULK -> {
                        linkState.incrementObservationBulkRxCount()
                    }
                }
            }
            else -> {}
        }
        frameListeners.forEach { it(frame) }
    }

    /**
     * Replay frames use awaited handlers; any persistence failure aborts confirmation.
     * Awaited replay listeners are owned by [SpoolReplayAssembler] (notified per inner frame).
     */
    private suspend fun dispatchReplayFrame(
        frame: ByteArray,
        sessionGeneration: Long,
    ) {
        when (val parsed = NativeCore.unpackPackLinkFrame(frame)) {
            is NativeCore.PackLinkFrameResult.Ok -> {
                when (parsed.msgType) {
                    MSG_TYPE_GATEWAY_INFO -> handleGatewayInfoBytes(frame)
                    MSG_TYPE_EVENT_NOTIFY -> eventNotifyHandler.onReplayEventNotifyFrame(frame)
                    MSG_TYPE_SPOOL_REPLAY -> spoolReplayAssembler.onSpoolReplayFrame(frame)
                    MSG_TYPE_TIME_SYNC -> handleTimeSyncBytes(frame)
                    MSG_TYPE_SNIFFER_STATUS -> handleSnifferStatusBytes(frame)
                    MSG_TYPE_CAPTURE_STATUS -> handleCaptureStatusBytes(frame)
                    MSG_TYPE_POLICY_SYNC_ACK -> handlePolicySyncAckBytes(frame, sessionGeneration)
                    MSG_TYPE_CHANNEL_PLAN_SYNC_ACK ->
                        handleChannelPlanSyncAckBytes(frame, sessionGeneration)
                    MSG_TYPE_OBSERVATION_BULK -> {
                        linkState.incrementObservationBulkRxCount()
                    }
                }
            }
            else -> throw IllegalArgumentException("invalid inner replay frame")
        }
        spoolReplayAssembler.notifyReplayFrame(frame)
    }

    private fun handleTimeSyncBytes(bytes: ByteArray) {
        val reply = NativeCore.unpackTimeSync(bytes) ?: return
        noteGatewayMono(reply.gatewayMonoMs)
    }

    private fun handlePolicySyncAckBytes(
        bytes: ByteArray,
        sessionGeneration: Long,
    ) {
        NativeCore.unpackPolicySyncAck(bytes)?.let { ack ->
            val accepted =
                synchronized(writerLock) {
                    expectedPolicyAckSeq == ack.policySeq &&
                        expectedPolicyAckGeneration == sessionGeneration &&
                        activeSessionGeneration == sessionGeneration &&
                        sessionGeneration != 0L
                }
            if (accepted) {
                linkState.setLastPolicySyncAck(ack)
            }
        }
    }

    private fun handleChannelPlanSyncAckBytes(
        bytes: ByteArray,
        sessionGeneration: Long,
    ) {
        NativeCore.unpackChannelPlanSyncAck(bytes)?.let { ack ->
            val accepted =
                synchronized(writerLock) {
                    expectedChannelPlanAckSeq == ack.planSeq &&
                        expectedChannelPlanAckGeneration == sessionGeneration &&
                        activeSessionGeneration == sessionGeneration &&
                        sessionGeneration != 0L
                }
            if (accepted) {
                linkState.setLastChannelPlanSyncAck(ack)
            }
        }
    }

    private fun handleSnifferStatusBytes(bytes: ByteArray) {
        val batch = NativeCore.unpackSnifferStatusBatch(bytes) ?: return
        if (batch.rows.isEmpty() && batch.gatewayMonoMs <= 0L) return
        noteGatewayMono(batch.gatewayMonoMs)
        linkState.setSnifferStatus(batch.rows)
        scope.launch {
            appContext.wardriverApp().fleetRepository.mergeSnifferStatus(batch.rows)
        }
    }

    private fun handleCaptureStatusBytes(bytes: ByteArray) {
        NativeCore.unpackCaptureStatus(bytes)?.let { linkState.setCaptureStatus(it) }
    }

    private fun handleFleetUnbindNotify() {
        scope.launch {
            val cleared = appContext.wardriverApp().fleetRepository.clearAll()
            android.util.Log.i(
                "GatewayWifi",
                "fleet unbind notify: cleared $cleared roster rows",
            )
        }
    }

    private fun handleGatewayInfoBytes(bytes: ByteArray) {
        parseGatewayInfo(bytes)?.let { linkState.setGatewayInfo(it) }
    }

    private fun parseGatewayInfo(wire: ByteArray): GatewayInfo? {
        val frame =
            when (val result = NativeCore.unpackPackLinkFrame(wire)) {
                is NativeCore.PackLinkFrameResult.Ok -> result
                else -> return null
            }
        if (frame.msgType != MSG_TYPE_GATEWAY_INFO) return null
        val payload = hexToBytes(frame.payloadHex)
        if (payload.size < GATEWAY_INFO_LEGACY_PAYLOAD_LEN) return null
        val mac = (0 until 6).joinToString(":") { i -> "%02X".format(payload[i].toInt() and 0xFF) }
        var offset = 6
        val firmwareBuild = readU16Le(payload, offset)
        offset += 2
        val packlinkVersion = payload[offset++].toInt() and 0xFF
        val protoVersion = payload[offset++].toInt() and 0xFF
        val spoolCapacity = readU32Le(payload, offset)
        offset += 4
        val spoolUsed = readU32Le(payload, offset)
        offset += 4
        val peerSlotsFree = payload[offset++].toInt() and 0xFF
        val peerJoinWaiters = payload[offset++].toInt() and 0xFF
        val fleetId = readU32Le(payload, offset)
        offset += 4
        val bootId = readU32Le(payload, offset)
        offset += 4
        val capabilities =
            if (payload.size >= GATEWAY_INFO_PAYLOAD_LEN) {
                payload[28].toInt() and 0xFF
            } else {
                0
            }
        return GatewayInfo(
            gatewayMac = mac,
            firmwareBuild = firmwareBuild,
            packlinkVersion = packlinkVersion,
            protoVersion = protoVersion,
            spoolCapacityBytes = spoolCapacity,
            spoolUsedBytes = spoolUsed,
            peerSlotsFree = peerSlotsFree,
            peerJoinWaiters = peerJoinWaiters,
            fleetId = fleetId,
            bootId = bootId,
            capabilities = capabilities,
        )
    }

    data class PendingTofu(
        val socket: Socket,
        val session: PackLinkTcpSession,
        val generation: Long,
    )

    private data class PendingWrite(
        val payload: ByteArray,
        val isPolicySync: Boolean = false,
        val isChannelPlanSync: Boolean = false,
        val generation: Long,
        val completion: CompletableDeferred<Unit>? = null,
    )

    sealed class ConnectState {
        data object Idle : ConnectState()
        data object Joining : ConnectState()
        data object AwaitingConfirmation : ConnectState()
        data class Failed(val message: String) : ConnectState()
    }

    sealed interface ConnectionState {
        data object Disconnected : ConnectionState
        data object Connecting : ConnectionState
        data object Reconnecting : ConnectionState
        data object Connected : ConnectionState
        data class Error(val message: String) : ConnectionState
    }

    data class GatewayInfo(
        val gatewayMac: String,
        val firmwareBuild: Int,
        val packlinkVersion: Int,
        val protoVersion: Int,
        val spoolCapacityBytes: Long,
        val spoolUsedBytes: Long,
        val peerSlotsFree: Int,
        val peerJoinWaiters: Int,
        val fleetId: Long,
        val bootId: Long,
        /** Optional capability tail (`gateway_capabilities::*`); `0` on legacy gateways. */
        val capabilities: Int = 0,
    )

    data class CaptureStatus(
        val active: Boolean,
        val hold: Boolean,
        val remainingMs: Long,
        val fragsRx: Long,
        val bytesWritten: Long,
        val errorCode: Int,
    )

    companion object {
        private const val MSG_TYPE_GATEWAY_INFO = 1
        private const val MSG_TYPE_EVENT_NOTIFY = 4
        private const val MSG_TYPE_OBSERVATION_BULK = 5
        private const val MSG_TYPE_SPOOL_REPLAY = 6
        private const val MSG_TYPE_TIME_SYNC = 7
        private const val MSG_TYPE_SNIFFER_STATUS = 8
        private const val MSG_TYPE_CAPTURE_STATUS = 10
        private const val MSG_TYPE_TRUST_OK = 12
        private const val MSG_TYPE_TRUST_REJECT = 13
        private const val MSG_TYPE_POLICY_SYNC_ACK = 16
        private const val MSG_TYPE_CHANNEL_PLAN_SYNC_ACK = 18
        private const val MSG_TYPE_FLEET_UNBIND_NOTIFY = 19
        private const val GATEWAY_INFO_LEGACY_PAYLOAD_LEN = 28
        private const val GATEWAY_INFO_PAYLOAD_LEN = 29
        private const val CAP_CHANNEL_PLAN_SYNC = 1 shl 0
        private const val RECONNECT_INITIAL_MS = 1_000L
        private const val RECONNECT_MAX_MS = 30_000L
        private const val RECONNECT_ATTEMPT_TIMEOUT_MS = 35_000L

        private fun readU16Le(bytes: ByteArray, offset: Int): Int =
            (bytes[offset].toInt() and 0xFF) or ((bytes[offset + 1].toInt() and 0xFF) shl 8)

        private fun readU32Le(bytes: ByteArray, offset: Int): Long =
            (bytes[offset].toLong() and 0xFF) or
                ((bytes[offset + 1].toLong() and 0xFF) shl 8) or
                ((bytes[offset + 2].toLong() and 0xFF) shl 16) or
                ((bytes[offset + 3].toLong() and 0xFF) shl 24)

        private fun hexToBytes(hex: String): ByteArray {
            val clean = hex.trim()
            if (clean.length < 2 || clean.length % 2 != 0) return ByteArray(0)
            return ByteArray(clean.length / 2) { i ->
                clean.substring(i * 2, i * 2 + 2).toInt(16).toByte()
            }
        }
    }
}
