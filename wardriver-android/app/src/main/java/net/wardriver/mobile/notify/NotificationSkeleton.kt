package net.wardriver.mobile.notify

import android.Manifest
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.RingtoneManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import net.wardriver.mobile.MainActivity

/**
 * Shared notification skeleton for detection and geofence alerts: permission check,
 * launch PendingIntent, base builder, and sound/vibration application.
 */
object NotificationSkeleton {
    /** Shared vibrate pattern for all alert notifications. */
    val VIBRATE_PATTERN: LongArray = longArrayOf(0, 200, 100, 200)

    /** True when the app may post notifications (POST_NOTIFICATIONS + user toggle). */
    fun canPostNotifications(context: Context): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (
                ContextCompat.checkSelfPermission(
                    context,
                    Manifest.permission.POST_NOTIFICATIONS,
                ) != PackageManager.PERMISSION_GRANTED
            ) {
                return false
            }
        }
        return NotificationManagerCompat.from(context).areNotificationsEnabled()
    }

    /** PendingIntent opening MainActivity (with optional extras, e.g. open_tab). */
    fun launchContentIntent(
        context: Context,
        extras: Map<String, String> = emptyMap(),
    ): PendingIntent {
        val launchIntent =
            Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                for ((key, value) in extras) putExtra(key, value)
            }
        return PendingIntent.getActivity(
            context,
            0,
            launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    /** Base builder: title, body, big text, alert icon, accent color, tap intent. */
    fun baseBuilder(
        context: Context,
        channelId: String,
        title: String,
        body: String,
        colorArgb: Int,
        contentIntent: PendingIntent,
    ): NotificationCompat.Builder =
        NotificationCompat.Builder(context, channelId)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setColor(colorArgb)
            .setContentIntent(contentIntent)
            .setAutoCancel(true)

    /**
     * Applies sound + vibration. [soundEnabled]/[vibrateEnabled] come from user prefs;
     * quiet delivery passes both false (silent, no vibration).
     */
    fun applySoundAndVibration(
        builder: NotificationCompat.Builder,
        soundEnabled: Boolean,
        vibrateEnabled: Boolean,
    ) {
        if (soundEnabled) {
            builder.setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION))
        } else {
            builder.setSilent(true)
        }
        if (vibrateEnabled) {
            builder.setVibrate(VIBRATE_PATTERN)
        }
    }
}
