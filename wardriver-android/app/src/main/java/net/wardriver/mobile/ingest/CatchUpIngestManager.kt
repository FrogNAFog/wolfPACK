package net.wardriver.mobile.ingest

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionKind
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.upload.UploadCoordinator

/**
 * Gateway-connected observation ingest when no foreground wardrive is active.
 * Registers PackLink listeners so spool replay cannot tear down the TCP read loop.
 */
class CatchUpIngestManager(
    private val db: WardriverDatabase,
    private val scope: CoroutineScope,
    private val hooks: Hooks,
) {
    interface Hooks {
        fun addFrameListener(listener: (ByteArray) -> Unit)
        fun removeFrameListener(listener: (ByteArray) -> Unit)
        fun addReplayFrameListener(listener: suspend (ByteArray) -> Unit)
        fun removeReplayFrameListener(listener: suspend (ByteArray) -> Unit)
        fun setEventNotifySessionId(sessionId: Long)
        fun currentGatewayMac(): String?
        fun isWardriveActive(): Boolean
    }

    private val lock = Mutex()
    private var ingestor: ObservationIngestor? = null
    private var sessionId: Long = 0

    private val packLinkFrameListener: (ByteArray) -> Unit = { frame ->
        ingestor?.onPackLinkFrame(frame)
    }

    private val replayPackLinkFrameListener: suspend (ByteArray) -> Unit = { frame ->
        ingestor?.onReplayPackLinkFrame(frame, failOnInvalid = false)
    }

    /** Start catch-up ingest when connected and wardrive is idle. */
    suspend fun ensureStarted() {
        if (UploadCoordinator.uploadInProgress) return
        if (hooks.isWardriveActive()) return
        lock.withLock {
            if (ingestor != null) return
            val gatewayMac = hooks.currentGatewayMac().orEmpty()
            val existing = db.sessionDao().getLatestOpenOfKind(SessionKind.CATCH_UP)
            val sid =
                if (existing != null && existing.gatewayMac == gatewayMac) {
                    existing.id
                } else {
                    db.sessionDao().insert(
                        SessionEntity(
                            startedUnixMs = System.currentTimeMillis(),
                            endedUnixMs = null,
                            gatewayMac = gatewayMac,
                            sessionKind = SessionKind.CATCH_UP,
                            geofenceLatMin = null,
                            geofenceLatMax = null,
                            geofenceLonMin = null,
                            geofenceLonMax = null,
                        ),
                    )
                }
            sessionId = sid
            val ing = ObservationIngestor(db = db, sessionId = sid, requireInitialFreshGps = false)
            ing.openSession()
            ingestor = ing
            hooks.setEventNotifySessionId(sid)
            hooks.addFrameListener(packLinkFrameListener)
            hooks.addReplayFrameListener(replayPackLinkFrameListener)
        }
    }

    /** Restart catch-up after upload when wardrive remains idle. */
    fun resumeAfterUploadIfNeeded() {
        scope.launch {
            ensureStarted()
        }
    }

    /** Promote catch-up session for wardrive when recent and gateway matches. */
    suspend fun promoteForWardriveWithIngestor(): Pair<Long, ObservationIngestor>? {
        return lock.withLock {
            val ing = ingestor ?: return@withLock null
            val sid = sessionId
            val session = db.sessionDao().get(sid) ?: return@withLock null
            val gatewayMac = hooks.currentGatewayMac().orEmpty()
            val ageMs = System.currentTimeMillis() - session.startedUnixMs
            if (session.sessionKind != SessionKind.CATCH_UP ||
                session.endedUnixMs != null ||
                ageMs > PROMOTE_MAX_AGE_MS ||
                (gatewayMac.isNotEmpty() &&
                    session.gatewayMac.isNotEmpty() &&
                    session.gatewayMac != gatewayMac)
            ) {
                return@withLock null
            }
            hooks.removeFrameListener(packLinkFrameListener)
            hooks.removeReplayFrameListener(replayPackLinkFrameListener)
            db.sessionDao().updateSessionKind(sid, SessionKind.WARDRIVE)
            if (gatewayMac.isNotEmpty()) {
                db.sessionDao().updateGatewayMac(sid, gatewayMac)
            }
            ingestor = null
            sessionId = 0
            sid to ing
        }
    }

    fun tearDown() {
        scope.launch {
            lock.withLock {
                tearDownLocked()
            }
        }
    }

    /** Synchronous tear-down for upload prep; returns whether catch-up was active. */
    suspend fun tearDownForUpload(): Boolean =
        lock.withLock {
            tearDownLocked()
        }

    private suspend fun tearDownLocked(): Boolean {
        val wasActive = ingestor != null
        val ing = ingestor
        ingestor = null
        hooks.removeFrameListener(packLinkFrameListener)
        hooks.removeReplayFrameListener(replayPackLinkFrameListener)
        if (ing != null && sessionId > 0) {
            ing.shutdown()
            val session = db.sessionDao().get(sessionId)
            if (session != null && session.endedUnixMs == null) {
                db.sessionDao().update(session.copy(endedUnixMs = System.currentTimeMillis()))
            }
            if (!hooks.isWardriveActive()) {
                hooks.setEventNotifySessionId(0)
            }
            sessionId = 0
        }
        return wasActive
    }

    fun isActive(): Boolean = ingestor != null

    companion object {
        /** Max catch-up session age eligible for wardrive promotion. */
        const val PROMOTE_MAX_AGE_MS = 30 * 60 * 1000L
    }
}
