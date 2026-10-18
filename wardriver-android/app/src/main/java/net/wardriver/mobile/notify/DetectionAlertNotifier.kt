package net.wardriver.mobile.notify

import android.content.Context
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.data.AlertDeliveryMode
import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.ingest.EventNotifyAlert
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Posts system notifications for detection alerts (shade, sound, vibration). */
object DetectionAlertNotifier {
    suspend fun postSystemNotification(
        context: Context,
        alert: EventNotifyAlert,
        mode: AlertDeliveryMode,
        suppressedCount: Int,
        notificationId: Int,
    ) {
        val isDetectionKind = AlertKind.isDetectionKind(alert.kind)
        if (!isDetectionKind && alert.kind != AlertKind.REMOTE_ID) return

        val appPrefs = context.wardriverApp().appPreferences
        if (!appPrefs.detectionAlertsEnabledFlow.first()) return

        if (!NotificationSkeleton.canPostNotifications(context)) {
            Log.w(TAG, "Detection alert suppressed: notifications not permitted")
            return
        }

        val soundEnabled = appPrefs.soundDetectionFlow.first()
        val vibrateEnabled = appPrefs.vibrateDetectionFlow.first()

        AlertChannels.ensureCreated(context.applicationContext)

        val contentIntent =
            NotificationSkeleton.launchContentIntent(context.applicationContext)

        val title = methodBasedTitle(alert)
        val body =
            if (suppressedCount > 0) {
                "${alert.detail} (+$suppressedCount suppressed)"
            } else {
                alert.detail
            }

        val channelId =
            when (mode) {
                AlertDeliveryMode.Full -> AlertChannels.ALERT_CHANNEL_ID
                AlertDeliveryMode.Quiet,
                AlertDeliveryMode.InAppWhenForeground,
                -> AlertChannels.ALERT_CHANNEL_QUIET_ID
            }

        val builder =
            NotificationSkeleton.baseBuilder(
                context = context,
                channelId = channelId,
                title = title,
                body = body,
                colorArgb = PackColor.argb(PackColor.Sun),
                contentIntent = contentIntent,
            )

        when (mode) {
            AlertDeliveryMode.Full -> {
                builder
                    .setPriority(NotificationCompat.PRIORITY_HIGH)
                    .setCategory(NotificationCompat.CATEGORY_ALARM)
                    .setOnlyAlertOnce(true)
                NotificationSkeleton.applySoundAndVibration(
                    builder,
                    soundEnabled,
                    vibrateEnabled,
                )
            }
            AlertDeliveryMode.Quiet,
            AlertDeliveryMode.InAppWhenForeground,
            -> {
                builder
                    .setPriority(NotificationCompat.PRIORITY_DEFAULT)
                    .setCategory(NotificationCompat.CATEGORY_STATUS)
                NotificationSkeleton.applySoundAndVibration(builder, false, false)
            }
        }

        NotificationManagerCompat.from(context).notify(notificationId, builder.build())
    }

    private const val TAG = "DetectionAlertNotifier"
}
