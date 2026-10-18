package net.wardriver.mobile.notify

import android.content.Context
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.data.AlertDeliveryMode
import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.ingest.EventNotifyAlert
import net.wardriver.mobile.wardriverApp

/**
 * One alert pipeline for all realtime detection alerts: kind filter -> enabled gate ->
 * per-MAC throttle -> delivery mode (in-app [AlertBus] vs system notification).
 *
 * Detection and Remote ID alerts are configurations of the same router:
 * [DetectionAlertRouter] and [RemoteIdAlertRouter].
 */
class AlertRouter(
    private val kindFilter: (String) -> Boolean,
    private val enabledFlow: (Context) -> Flow<Boolean>,
    private val notificationIdOffset: Int = 0,
    private val notificationTitle: (EventNotifyAlert) -> String,
) {
    suspend fun route(context: Context, alert: EventNotifyAlert) {
        if (!kindFilter(alert.kind)) return

        val appPrefs = context.wardriverApp().appPreferences
        if (!enabledFlow(context).first()) return

        val mac = AlertThrottle.parseMac(alert.detail)
        val throttleKey = AlertThrottle.throttleKey(alert.kind, mac)
        val cooldownMs = appPrefs.detectionNotifyCooldownMsFlow.first()

        val decision =
            if (mac != null) {
                AlertThrottle.shouldNotify(throttleKey, alert.unixMs, cooldownMs)
            } else {
                AlertThrottle.shouldNotify(
                    key = "$throttleKey:${alert.unixMs}",
                    nowMs = alert.unixMs,
                    cooldownMs = cooldownMs,
                    skipThrottle = true,
                )
            }
        if (decision is NotifyDecision.Suppress) return

        val suppressedCount = (decision as NotifyDecision.Post).suppressedCount
        val deliveryMode = appPrefs.detectionDeliveryModeFlow.first()
        val foreground =
            ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)

        val notificationId =
            mac?.let { AlertThrottle.stableNotificationId(it) + notificationIdOffset }
                ?: AlertThrottle.fallbackNotificationId(alert.kind, alert.unixMs) +
                    notificationIdOffset

        suspend fun postQuiet() {
            DetectionAlertNotifier.postSystemNotification(
                context = context,
                alert = alert,
                mode = AlertDeliveryMode.Quiet,
                suppressedCount = suppressedCount,
                notificationId = notificationId,
            )
        }
        when {
            deliveryMode == AlertDeliveryMode.InAppWhenForeground && foreground -> {
                AlertBus.emit(
                    kind = AlertBus.Kind.Detection,
                    title = notificationTitle(alert),
                    detail = alert.detail,
                    suppressedCount = suppressedCount,
                )
            }
            deliveryMode == AlertDeliveryMode.InAppWhenForeground -> postQuiet()
            deliveryMode == AlertDeliveryMode.Full -> {
                DetectionAlertNotifier.postSystemNotification(
                    context = context,
                    alert = alert,
                    mode = AlertDeliveryMode.Full,
                    suppressedCount = suppressedCount,
                    notificationId = notificationId,
                )
            }
            else -> postQuiet()
        }
    }
}

/** Method-based notification title (Axon / i-PRO take priority over kind). */
internal fun methodBasedTitle(alert: EventNotifyAlert): String =
    when (alert.detectionMethodId) {
        4, 35 -> "Axon alert"
        5, 36 -> "i-PRO alert"
        else ->
            when (alert.kind) {
                in AlertKind.WIFI_KINDS -> "Camera WiFi alert"
                in AlertKind.BLE_KINDS -> "Detection alert"
                AlertKind.REMOTE_ID -> "Remote ID alert"
                else -> "Detection alert"
            }
    }

private val detectionEnabledFlow: (Context) -> Flow<Boolean> =
    { context -> context.wardriverApp().appPreferences.detectionAlertsEnabledFlow }

/** Camera detection alerts (legacy FLOCK_* + renamed DETECTION_*). */
object DetectionAlertRouter {
    private val router =
        AlertRouter(
            kindFilter = { kind -> AlertKind.DETECTION_KINDS.contains(kind) },
            enabledFlow = detectionEnabledFlow,
            notificationTitle = ::methodBasedTitle,
        )

    suspend fun onDetectionAlert(context: Context, alert: EventNotifyAlert) {
        router.route(context, alert)
    }
}

/** Remote ID (drone) alerts. */
object RemoteIdAlertRouter {
    private val router =
        AlertRouter(
            kindFilter = { kind -> kind == AlertKind.REMOTE_ID },
            enabledFlow = detectionEnabledFlow,
            notificationIdOffset = AlertChannels.REMOTE_ID_NOTIFICATION_OFFSET,
            notificationTitle = { _ -> "Remote ID alert" },
        )

    suspend fun onRemoteIdAlert(context: Context, alert: EventNotifyAlert) {
        router.route(context, alert)
    }
}
