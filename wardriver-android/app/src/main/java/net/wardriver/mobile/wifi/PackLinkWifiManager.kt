package net.wardriver.mobile.wifi

import android.content.BroadcastReceiver
import android.content.Context
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.net.wifi.WifiNetworkSpecifier
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.annotation.RequiresApi
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * Join the gateway management SoftAP without making it the default internet network.
 *
 * Joins in three stages, preferring the OS's own join so the app never creates a
 * second (ephemeral) STA on the same BSSID — which Android tears down on
 * single-STA devices, causing a deauth flap:
 *
 * 1. The phone's default Wi-Fi is already the target SoftAP → open a plain socket.
 * 2. Otherwise, give the OS a short grace window to auto-join a saved SoftAP,
 *    then open a plain socket on the default network.
 * 3. Only when the OS will not auto-join (first-time join, or the gateway is a
 *    secondary network on a dual-STA phone) fall back to an ephemeral STA via
 *    `requestNetwork`.
 *
 * The join sequence runs on a single background executor so the caller (a
 * Main-scope coroutine) is never blocked; the callbacks are delivered on the
 * main thread.
 */
class PackLinkWifiManager(private val context: Context) {
    private val connectivity =
        context.applicationContext.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private val wifiManager =
        context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    private val activeCallback = AtomicReference<ConnectivityManager.NetworkCallback?>(null)
    private val activeNetwork = AtomicReference<Network?>(null)
    private val joinExecutor = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())
    @Volatile
    private var wifiLock: WifiManager.WifiLock? = null
    @Volatile
    private var defaultSocket: Socket? = null
    @Volatile
    private var defaultReceiver: BroadcastReceiver? = null

    fun connectToSoftAp(
        ssid: String,
        password: String,
        onReady: (Socket) -> Unit,
        onFailed: (String) -> Unit,
        onLinkLost: (() -> Unit)? = null,
    ) {
        release()
        joinExecutor.execute {
            runJoinSequence(ssid, password, onReady, onFailed, onLinkLost)
        }
    }

    /** Re-join SoftAP after link loss; keeps the WifiLock held across attempts. */
    fun reconnectToSoftAp(
        ssid: String,
        password: String,
        onReady: (Socket) -> Unit,
        onFailed: (String) -> Unit,
        onLinkLost: (() -> Unit)? = null,
    ) {
        releaseNetwork()
        joinExecutor.execute {
            runJoinSequence(ssid, password, onReady, onFailed, onLinkLost)
        }
    }

    /**
     * The full join sequence (stages 1 → 2 → 3), run on the background executor.
     * Callbacks are re-dispatched to the main thread before being invoked.
     */
    private fun runJoinSequence(
        ssid: String,
        password: String,
        onReady: (Socket) -> Unit,
        onFailed: (String) -> Unit,
        onLinkLost: (() -> Unit)?,
    ) {
        val ready = { socket: Socket -> postOnMain { onReady(socket) } }
        val failed = { message: String -> postOnMain { onFailed(message) } }
        val linkLost: (() -> Unit)? = onLinkLost?.let { original -> { postOnMain { original() } } }

        // Stage 1: the default network is already the target SoftAP.
        if (isDefaultNetworkOnSoftAp(ssid)) {
            connectDefaultNetwork(ssid, ready, failed, linkLost)
            return
        }

        // Stage 2: a saved SoftAP auto-joins within a couple of seconds; wait for
        // it so we never create a second STA on the same BSSID.
        if (waitForAutoJoin(ssid)) {
            connectDefaultNetwork(ssid, ready, failed, linkLost)
            return
        }

        // Stage 3: genuine fallback — first-time join, or a secondary network on a
        // dual-STA phone. requestNetwork is Q+.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            failed("Wi-Fi join requires Android 10+")
            return
        }
        connectApi29(ssid, password, ready, failed, linkLost)
    }

    /**
     * Fast path: the phone's default Wi-Fi network is already the target SoftAP.
     * Opens a plain socket on the default network without a second (ephemeral) STA,
     * which Android tears down on dual-STA-unsupported devices.
     */
    private fun connectDefaultNetwork(
        ssid: String,
        onReady: (Socket) -> Unit,
        onFailed: (String) -> Unit,
        onLinkLost: (() -> Unit)?,
    ) {
        acquireWifiLock()
        val socket = Socket()
        socket.tcpNoDelay = true
        try {
            socket.connect(
                InetSocketAddress(GATEWAY_HOST, PACKLINK_TCP_PORT),
                TCP_CONNECT_TIMEOUT_MS,
            )
        } catch (e: Exception) {
            socket.close()
            onFailed(e.message ?: "default-network TCP connect failed")
            return
        }
        defaultSocket = socket
        val lostNotified = AtomicBoolean(false)
        val receiver =
            object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: android.content.Intent) {
                    if (intent.action != ConnectivityManager.CONNECTIVITY_ACTION) return
                    val current = currentWifiSsid()
                    if (current == null || current != ssid) {
                        defaultSocket?.close()
                        defaultSocket = null
                        if (lostNotified.compareAndSet(false, true)) {
                            onLinkLost?.invoke()
                        }
                    }
                }
            }
        defaultReceiver = receiver
        context.applicationContext.registerReceiver(
            receiver,
            IntentFilter(ConnectivityManager.CONNECTIVITY_ACTION),
        )
        onReady(socket)
    }

    /**
     * Stage 2 grace: poll [currentWifiSsid] for up to [AUTO_JOIN_TIMEOUT_MS] total,
     * checking every [AUTO_JOIN_POLL_INTERVAL_MS], for it to become [ssid]. Returns
     * true as soon as it matches; false if the window elapses.
     */
    private fun waitForAutoJoin(ssid: String): Boolean {
        val deadline = SystemClock.elapsedRealtime() + AUTO_JOIN_TIMEOUT_MS
        while (SystemClock.elapsedRealtime() < deadline) {
            Thread.sleep(AUTO_JOIN_POLL_INTERVAL_MS)
            if (currentWifiSsid() == ssid) return true
        }
        return false
    }

    /**
     * The currently connected Wi-Fi SSID, or null when it cannot be determined
     * (e.g. location permission denied → Android returns `<unknown ssid>`).
     */
    private fun currentWifiSsid(): String? {
        @Suppress("DEPRECATION")
        val ssid = wifiManager.connectionInfo?.ssid ?: return null
        if (ssid == "<unknown ssid>") return null
        return ssid
    }

    /** True when the phone's default Wi-Fi network is exactly [targetSsid]. */
    private fun isDefaultNetworkOnSoftAp(targetSsid: String): Boolean {
        val current = currentWifiSsid() ?: return false
        return current == targetSsid
    }

    /** Deliver [action] on the main thread. */
    private fun postOnMain(action: () -> Unit) {
        mainHandler.post { action() }
    }

    fun releaseNetwork() {
        activeCallback.getAndSet(null)?.let { connectivity.unregisterNetworkCallback(it) }
        activeNetwork.set(null)
        defaultReceiver?.let {
            context.applicationContext.unregisterReceiver(it)
        }
        defaultReceiver = null
        defaultSocket?.let { socket ->
            try {
                socket.close()
            } catch (e: Exception) {
                // ignore
            }
        }
        defaultSocket = null
    }

    fun release() {
        releaseNetwork()
        releaseWifiLock()
    }

    private fun acquireWifiLock() {
        if (wifiLock?.isHeld == true) return
        @Suppress("DEPRECATION")
        val lock =
            wifiManager.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, WIFI_LOCK_TAG)
        lock.setReferenceCounted(false)
        lock.acquire()
        wifiLock = lock
    }

    private fun releaseWifiLock() {
        wifiLock?.let { lock ->
            if (lock.isHeld) {
                lock.release()
            }
        }
        wifiLock = null
    }

    @RequiresApi(Build.VERSION_CODES.Q)
    private fun connectApi29(
        ssid: String,
        password: String,
        onReady: (Socket) -> Unit,
        onFailed: (String) -> Unit,
        onLinkLost: (() -> Unit)?,
    ) {
        val specifier =
            WifiNetworkSpecifier.Builder()
                .setSsid(ssid)
                .setWpa2Passphrase(password)
                .build()
        val request =
            NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .setNetworkSpecifier(specifier)
                .build()
        val callback =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    activeNetwork.set(network)
                    acquireWifiLock()
                    try {
                        val socket =
                            network.socketFactory.createSocket() as Socket
                        socket.connect(
                            InetSocketAddress(GATEWAY_HOST, PACKLINK_TCP_PORT),
                            TCP_CONNECT_TIMEOUT_MS,
                        )
                        onReady(socket)
                    } catch (e: Exception) {
                        onFailed(e.message ?: "TCP connect failed")
                    }
                }

                override fun onUnavailable() {
                    onFailed("Could not join gateway Wi-Fi")
                }

                override fun onLost(network: Network) {
                    if (activeNetwork.compareAndSet(network, null)) {
                        onLinkLost?.invoke()
                    }
                }
            }
        activeCallback.set(callback)
        connectivity.requestNetwork(request, callback, HANDSHAKE_TIMEOUT_MS)
    }

    companion object {
        const val GATEWAY_HOST = "192.168.71.1"
        const val PACKLINK_TCP_PORT = 4710
        private const val WIFI_LOCK_TAG = "wolfPACK:gateway"
        private const val TCP_CONNECT_TIMEOUT_MS = 10_000
        private const val HANDSHAKE_TIMEOUT_MS = 30_000
        private const val AUTO_JOIN_POLL_INTERVAL_MS = 500L
        private const val AUTO_JOIN_TIMEOUT_MS = 5_000L
    }
}
