package net.wardriver.mobile.notify

import android.content.Context
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.flow.first
import net.wardriver.mobile.ui.nav.AppDestinations
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.wardriverApp

/** Posts PACK-themed in-app and system notifications for geofence enter/leave. */
object GeofenceTransitionNotifier {
    enum class Transition {
        Enter,
        Leave,
    }

    suspend fun notify(
        context: Context,
        transition: Transition,
    ) {
        val appPrefs = context.wardriverApp().appPreferences
        if (!appPrefs.geofenceAlertsEnabledFlow.first()) return

        val (title, subtitle) = copyFor(transition)
        val kind =
            when (transition) {
                Transition.Enter -> AlertBus.Kind.GeofenceEnter
                Transition.Leave -> AlertBus.Kind.GeofenceLeave
            }
        AlertBus.emit(
            kind = kind,
            title = title,
            detail = subtitle,
            suppressedCount = 0,
        )
        postSystemNotification(context, transition, title, subtitle)
    }

    private fun copyFor(transition: Transition): Pair<String, String> =
        when (transition) {
            Transition.Enter ->
                "Inside wardrive square" to "Wardrive geofence active"
            Transition.Leave ->
                "Left wardrive square" to "Return to your geofence to continue wardriving"
        }

    private suspend fun postSystemNotification(
        context: Context,
        transition: Transition,
        title: String,
        body: String,
    ) {
        if (!NotificationSkeleton.canPostNotifications(context)) {
            Log.w(TAG, "Geofence alert suppressed: notifications not permitted")
            return
        }

        val appPrefs = context.wardriverApp().appPreferences
        val soundEnabled = appPrefs.soundDetectionFlow.first()
        val vibrateEnabled = appPrefs.vibrateDetectionFlow.first()

        AlertChannels.ensureCreated(context.applicationContext)

        val contentIntent =
            NotificationSkeleton.launchContentIntent(
                context.applicationContext,
                extras = mapOf(EXTRA_OPEN_TAB to AppDestinations.Map.route),
            )

        val accent =
            when (transition) {
                Transition.Enter -> PackColor.argb(PackColor.Good)
                Transition.Leave -> PackColor.argb(PackColor.Sun)
            }
        val notificationId =
            when (transition) {
                Transition.Enter -> AlertChannels.GEOFENCE_ENTER_NOTIFICATION_ID
                Transition.Leave -> AlertChannels.GEOFENCE_LEAVE_NOTIFICATION_ID
            }

        val builder =
            NotificationSkeleton.baseBuilder(
                context = context,
                channelId = AlertChannels.GEOFENCE_CHANNEL_ID,
                title = title,
                body = body,
                colorArgb = accent,
                contentIntent = contentIntent,
            )
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_STATUS)

        NotificationSkeleton.applySoundAndVibration(
            builder,
            soundEnabled,
            vibrateEnabled,
        )

        NotificationManagerCompat.from(context).notify(notificationId, builder.build())
    }

    const val EXTRA_OPEN_TAB = "open_tab"

    private const val TAG = "GeofenceTransitionNotifier"
}
