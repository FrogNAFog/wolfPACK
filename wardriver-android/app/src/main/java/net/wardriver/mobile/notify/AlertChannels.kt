package net.wardriver.mobile.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import net.wardriver.mobile.ui.theme.PackColor

/** Shared notification channels for wardrive detection alerts. */
object AlertChannels {
    const val ALERT_CHANNEL_ID = "wardrive_alerts"
    const val ALERT_CHANNEL_QUIET_ID = "wardrive_alerts_quiet"
    const val GEOFENCE_CHANNEL_ID = "wardrive_geofence"
    const val GEOFENCE_LEAVE_NOTIFICATION_ID = 43
    const val GEOFENCE_ENTER_NOTIFICATION_ID = 44
    const val DETECTION_NOTIFICATION_BASE_ID = 1000
    const val REMOTE_ID_NOTIFICATION_OFFSET = 2000

    @Deprecated("Use GEOFENCE_LEAVE_NOTIFICATION_ID", ReplaceWith("GEOFENCE_LEAVE_NOTIFICATION_ID"))
    const val GEOFENCE_NOTIFICATION_ID = GEOFENCE_LEAVE_NOTIFICATION_ID

    fun ensureCreated(context: Context) {
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(
                ALERT_CHANNEL_ID,
                "Detection alerts",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                lightColor = PackColor.argb(PackColor.Sun)
            },
        )
        nm.createNotificationChannel(
            NotificationChannel(
                ALERT_CHANNEL_QUIET_ID,
                "Detection alerts (quiet)",
                NotificationManager.IMPORTANCE_DEFAULT,
            ).apply {
                lightColor = PackColor.argb(PackColor.Sun)
            },
        )
        nm.createNotificationChannel(
            NotificationChannel(
                GEOFENCE_CHANNEL_ID,
                "Wardrive geofence",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                lightColor = PackColor.argb(PackColor.Sun)
            },
        )
    }
}
