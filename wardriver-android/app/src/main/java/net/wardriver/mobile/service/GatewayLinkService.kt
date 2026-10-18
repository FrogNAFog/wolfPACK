package net.wardriver.mobile.service

import android.app.ForegroundServiceStartNotAllowedException
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import net.wardriver.mobile.MainActivity
import net.wardriver.mobile.WardriverApp
import net.wardriver.mobile.data.GatewayWifiRepository

/**
 * Foreground holder for an active gateway PackLink link (independent of wardrive).
 *
 * Defers its notification while [WardriveSessionService] owns foreground priority.
 */
class GatewayLinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var wardriveWatchJob: Job? = null
    private var connectionWatchJob: Job? = null
    private var foregroundHeld = false

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_DISCONNECT -> {
                (application as WardriverApp).gatewayWifiRepository.disconnect()
                stopGatewayForeground()
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_STOP -> {
                stopGatewayForeground()
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_REFRESH,
            null,
            -> {
                val gatewayWifi = (application as WardriverApp).gatewayWifiRepository
                val foregroundRequired =
                    intent?.getBooleanExtra(EXTRA_FOREGROUND_REQUIRED, false) == true
                wardriveWatchJob?.cancel()
                wardriveWatchJob =
                    scope.launch {
                        WardriveSessionService.sessionIdFlow.collectLatest {
                            updateForegroundState()
                        }
                    }
                connectionWatchJob?.cancel()
                connectionWatchJob =
                    scope.launch {
                        gatewayWifi.connectionState.collectLatest {
                            updateForegroundState()
                        }
                    }
                // startForegroundService() requires startForeground() even if wardrive demotes us next.
                satisfyForegroundContract(foregroundRequired)
                updateForegroundState()
            }
        }
        return START_STICKY
    }

    private fun updateForegroundState() {
        val wardriveActive = WardriveSessionService.sessionIdFlow.value != null
        wardriveActiveFlag = wardriveActive
        if (wardriveActive) {
            if (foregroundHeld) {
                stopGatewayForeground()
            }
            return
        }
        val gatewayWifi = (application as WardriverApp).gatewayWifiRepository
        val linkActive =
            when (gatewayWifi.connectionState.value) {
                GatewayWifiRepository.ConnectionState.Connected,
                GatewayWifiRepository.ConnectionState.Reconnecting,
                -> true
                else -> false
            }
        if (!linkActive) {
            stopGatewayForeground()
            stopSelf()
            return
        }
        promoteForeground(gatewaySubtitle(gatewayWifi))
    }

    /** Android requires startForeground() after startForegroundService() even if wardrive owns UI. */
    private fun satisfyForegroundContract(foregroundRequired: Boolean) {
        if (foregroundHeld) return
        if (!foregroundRequired) return
        val gatewayWifi = (application as WardriverApp).gatewayWifiRepository
        promoteForeground(gatewaySubtitle(gatewayWifi))
    }

    private fun gatewaySubtitle(gatewayWifi: GatewayWifiRepository): String =
        when (gatewayWifi.connectionState.value) {
            GatewayWifiRepository.ConnectionState.Reconnecting -> "Reconnecting to gateway..."
            else -> "Connected to wolfPACK gateway"
        }

    private fun promoteForeground(subtitle: String) {
        val notification = buildNotification(subtitle)
        val fgType = ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIFICATION_ID, notification, fgType)
            } else {
                @Suppress("DEPRECATION")
                startForeground(NOTIFICATION_ID, notification)
            }
            foregroundHeld = true
        } catch (e: Exception) {
            Log.e(TAG, "Gateway foreground start failed: ${e.message}", e)
            stopSelf()
        }
    }

    private fun stopGatewayForeground() {
        if (!foregroundHeld) return
        foregroundHeld = false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
    }

    private fun buildNotification(subtitle: String): Notification {
        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val disconnectIntent =
            PendingIntent.getService(
                this,
                1,
                Intent(this, GatewayLinkService::class.java).setAction(ACTION_DISCONNECT),
                PendingIntent.FLAG_IMMUTABLE,
            )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("wolfPACK gateway")
            .setContentText(subtitle)
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(0, "Disconnect", disconnectIntent)
            .build()
    }

    private fun createChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Gateway link", NotificationManager.IMPORTANCE_LOW),
        )
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        wardriveWatchJob?.cancel()
        connectionWatchJob?.cancel()
        scope.cancel()
        stopGatewayForeground()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "GatewayLinkService"
        const val CHANNEL_ID = "gateway_link"
        const val NOTIFICATION_ID = 44
        const val ACTION_DISCONNECT = "net.wardriver.mobile.GATEWAY_DISCONNECT"
        const val ACTION_STOP = "net.wardriver.mobile.GATEWAY_STOP"
        const val ACTION_REFRESH = "net.wardriver.mobile.GATEWAY_REFRESH"
        private const val EXTRA_FOREGROUND_REQUIRED = "foreground_required"

        @Volatile
        var wardriveActiveFlag: Boolean = false
            private set

        /** Start or refresh the gateway link foreground holder. */
        fun ensureRunning(context: Context) {
            val wardriveActive = WardriveSessionService.sessionIdFlow.value != null
            val intent = Intent(context, GatewayLinkService::class.java)
            try {
                if (wardriveActive) {
                    // WardriveSessionService owns FGS; avoid a second startForegroundService().
                    context.startService(intent)
                } else {
                    context.startForegroundService(
                        intent.putExtra(EXTRA_FOREGROUND_REQUIRED, true),
                    )
                }
            } catch (e: Exception) {
                if (Build.VERSION.SDK_INT >= 31 && e is ForegroundServiceStartNotAllowedException) {
                    Log.w(TAG, "Background gateway FGS blocked; WifiLock/reconnect still active")
                } else {
                    Log.w(TAG, "Gateway FGS start failed: ${e.message}")
                }
            }
        }

        /** Tear down gateway foreground when the user disconnects. */
        fun stop(context: Context) {
            context.startService(
                Intent(context, GatewayLinkService::class.java).setAction(ACTION_STOP),
            )
        }

        /** Wardrive session started or ended; service re-evaluates foreground ownership. */
        fun notifyWardriveActive(
            context: Context,
            active: Boolean,
        ) {
            wardriveActiveFlag = active
            // Refresh an existing holder only; never startForegroundService from wardrive teardown.
            context.startService(
                Intent(context, GatewayLinkService::class.java).setAction(ACTION_REFRESH),
            )
        }
    }
}
