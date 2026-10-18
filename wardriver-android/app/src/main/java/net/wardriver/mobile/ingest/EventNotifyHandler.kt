package net.wardriver.mobile.ingest

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.db.DetectionEntity
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.notify.DetectionAlertRouter
import net.wardriver.mobile.notify.RemoteIdAlertRouter
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.ui.map.isPlausibleWallUnixMs

/** Ingests PackLink `EventNotify` alerts into Room + detection system notifications. */
class EventNotifyHandler(
    private val scope: CoroutineScope,
    private val context: Context,
    private val database: WardriverDatabase,
) {
    @Volatile
    var sessionId: Long = 0

    fun onEventNotifyFrame(frame: ByteArray) {
        val alert = NativeCore.unpackEventNotify(frame) ?: return
        scope.launch { persist(alert, notifyUser = true, attachPhoneGps = true) }
    }

    /** Persist a replayed event before the replay acknowledgement is eligible to send. */
    suspend fun onReplayEventNotifyFrame(frame: ByteArray) {
        val alert =
            NativeCore.unpackEventNotify(frame)
                ?: throw IllegalArgumentException("invalid event replay payload")
        persist(alert, notifyUser = false, attachPhoneGps = false)
    }

    private suspend fun persist(
        alert: EventNotifyAlert,
        notifyUser: Boolean,
        attachPhoneGps: Boolean,
    ) = withContext(Dispatchers.IO) {
        val (lat, lon) = resolveDetectionCoords(alert, attachPhoneGps)
        val unixMs =
            alert.unixMs.takeIf { isPlausibleWallUnixMs(it) } ?: System.currentTimeMillis()
        database.detectionDao().insert(
            DetectionEntity(
                sessionId = sessionId,
                kind = alert.kind,
                unixMs = unixMs,
                detail = alert.detail,
                lat = lat,
                lon = lon,
                detectionMethodId = alert.detectionMethodId?.takeIf { it > 0 },
            ),
        )
        if (notifyUser && AlertKind.DETECTION_KINDS.contains(alert.kind)) {
            DetectionAlertRouter.onDetectionAlert(context, alert)
        }
        if (notifyUser && alert.kind == AlertKind.REMOTE_ID) {
            RemoteIdAlertRouter.onRemoteIdAlert(context, alert)
        }
    }

    private fun resolveDetectionCoords(
        alert: EventNotifyAlert,
        attachPhoneGps: Boolean,
    ): Pair<Double?, Double?> {
        if (alert.lat != null && alert.lon != null) {
            return alert.lat to alert.lon
        }
        if (!attachPhoneGps) {
            return null to null
        }
        val loc = WardriveSessionService.wardriveGpsFlow.value ?: return null to null
        if (!loc.hasAccuracy() || loc.accuracy > ObservationIngestor.ROUTE_MAX_ACCURACY_M) {
            return null to null
        }
        return loc.latitude to loc.longitude
    }
}

/** Parsed realtime alert from JNI. */
data class EventNotifyAlert(
    val kind: String,
    val snifferId: Int,
    val unixMs: Long,
    val detail: String,
    val lat: Double?,
    val lon: Double?,
    val detectionMethodId: Int? = null,
)
