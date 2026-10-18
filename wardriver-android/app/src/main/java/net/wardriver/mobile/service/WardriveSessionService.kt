package net.wardriver.mobile.service

import android.app.ForegroundServiceStartNotAllowedException
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import com.google.android.gms.tasks.CancellationTokenSource
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import net.wardriver.mobile.MainActivity
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.WardriverApp
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.DenylistBssids
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.data.HomeZoneSuppressor
import net.wardriver.mobile.data.LocationSource
import net.wardriver.mobile.data.PolicySyncAssembler
import net.wardriver.mobile.notify.AlertChannels
import net.wardriver.mobile.notify.GeofenceTransitionNotifier
import net.wardriver.mobile.service.GatewayLinkService
import net.wardriver.mobile.ui.map.GeofenceBounds
import net.wardriver.mobile.db.RoutePointEntity
import net.wardriver.mobile.db.SessionEntity
import net.wardriver.mobile.db.SessionKind
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.ingest.CellTowerCollector
import net.wardriver.mobile.ingest.GeofenceMonitor
import net.wardriver.mobile.ingest.GpsFreshness
import net.wardriver.mobile.ingest.GpsTimeline
import net.wardriver.mobile.ingest.ObservationIngestStats
import net.wardriver.mobile.ingest.ObservationIngestor
import net.wardriver.mobile.ingest.WardriveClock
import net.wardriver.mobile.ui.dashboard.WardriveStartPreflight
import net.wardriver.mobile.upload.UploadCoordinator
import net.wardriver.mobile.upload.UploadScheduler
import net.wardriver.mobile.wardriverApp

/** Foreground wardrive session (WiGLE WigleService pattern). */
class WardriveSessionService : Service() {
    private val scope = CoroutineScope(Dispatchers.Default + Job())
    private lateinit var db: WardriverDatabase
    private lateinit var gatewayWifi: GatewayWifiRepository
    private var sessionId: Long = 0
    private var ingestor: ObservationIngestor? = null
    private var notificationJob: Job? = null
    private var policyPushJob: Job? = null
    private var geofenceMonitor: GeofenceMonitor? = null
    private var paused = false
    private val homeZoneSuppressor = HomeZoneSuppressor()
    private var lastWardriveSuppressed: Boolean? = null
    private var lastGpsLat: Double? = null
    private var lastGpsLon: Double? = null
    private var lastGpsOk = false
    private var lastCellScanUnixMs = 0L
    private var lastLoggedGpsSource: GpsSource? = null
    private val cellCollector by lazy { CellTowerCollector(this) }

    /** Forwards PackLink frames to the active wardrive ingestor. */
    private val packLinkFrameListener: (ByteArray) -> Unit = { frame ->
        ingestor?.onPackLinkFrame(frame)
    }

    /** Awaits replay persistence so the gateway acknowledgement cannot overtake Room. */
    private val replayPackLinkFrameListener: suspend (ByteArray) -> Unit = { frame ->
        val active = ingestor ?: throw IllegalStateException("wardrive session ended during replay")
        active.onReplayPackLinkFrame(frame)
    }

    private val locationCallback =
        object : LocationCallback() {
            override fun onLocationResult(result: LocationResult) {
                val locations = result.locations.sortedBy { it.time }
                for (loc in locations) {
                    deliverLocationFix(loc, GpsSource.FUSED)
                }
            }
        }

    /** GPS_PROVIDER wakes GNSS; position ingest when accuracy is within route cap. */
    private val gpsProviderListener =
        LocationListener { loc -> deliverLocationFix(loc, GpsSource.GPS_PROVIDER) }

    override fun onCreate() {
        super.onCreate()
        db = WardriverDatabase.getInstance(this)
        gatewayWifi = (application as WardriverApp).gatewayWifiRepository
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val state = _startStateFlow.value
                if (state is WardriveSessionStartState.Running) {
                    return START_STICKY
                }
                _startStateFlow.value = WardriveSessionStartState.Starting
                if (!ensureForegroundStarted("Connecting gateway...")) {
                    return START_NOT_STICKY
                }
                scope.launch {
                    try {
                        beginWardriveSession(insertNewSession = true)
                    } catch (e: Exception) {
                        Log.e(TAG, "wardrive session start failed", e)
                        failSessionStart("Wardrive failed: ${e.message ?: e.javaClass.simpleName}")
                    }
                }
            }
            ACTION_PAUSE -> {
                paused = true
                ingestor?.setPaused(true)
                scope.launch { updateNotification() }
            }
            ACTION_RESUME -> {
                paused = false
                ingestor?.setPaused(false)
                scope.launch { updateNotification() }
            }
            ACTION_UPLOAD -> UploadScheduler.scheduleAll(this)
            ACTION_PREPARE_FOR_UPLOAD -> {
                scope.launch {
                    try {
                        val resume = prepareForUpload()
                        UploadCoordinator.completePrepare(resume)
                    } catch (e: Exception) {
                        UploadCoordinator.uploadInProgress = false
                        UploadCoordinator.failPrepare()
                    }
                }
            }
            ACTION_RESUME_AFTER_UPLOAD -> {
                scope.launch { resumeAfterUpload() }
            }
            ACTION_CONFIGURE_GEOFENCE -> {
                val latMin = intent.getDoubleExtra(EXTRA_LAT_MIN, Double.NaN)
                val latMax = intent.getDoubleExtra(EXTRA_LAT_MAX, Double.NaN)
                val lonMin = intent.getDoubleExtra(EXTRA_LON_MIN, Double.NaN)
                val lonMax = intent.getDoubleExtra(EXTRA_LON_MAX, Double.NaN)
                if (latMin.isFinite() && latMax.isFinite() && lonMin.isFinite() && lonMax.isFinite()) {
                    applyGeofence(latMin, latMax, lonMin, lonMax, enabled = true)
                }
            }
            ACTION_CLEAR_GEOFENCE -> applyGeofence(clear = true)
            ACTION_STOP -> {
                stopSession()
                return START_NOT_STICKY
            }
            null -> {
                if (!ensureForegroundStarted("Connecting gateway...")) {
                    return START_NOT_STICKY
                }
                scope.launch {
                    val app = application as WardriverApp
                    val prefs = app.appPreferences
                    if (prefs.wardriveActive()) {
                        val persistedId = prefs.activeWardriveSessionId()
                        val state = _startStateFlow.value
                        if (persistedId != null &&
                            state !is WardriveSessionStartState.Starting &&
                            state !is WardriveSessionStartState.Running
                        ) {
                            _startStateFlow.value = WardriveSessionStartState.Starting
                            try {
                                resumeSession(persistedId)
                            } catch (e: Exception) {
                                Log.e(TAG, "wardrive session restore failed", e)
                                failSessionStart("Wardrive restore failed: ${e.message ?: e.javaClass.simpleName}")
                            }
                        }
                    }
                }
            }
            else -> Unit
        }
        return START_STICKY
    }

    /** Tear down foreground wardrive: end session row and release gateway ingest listener. */
    private fun stopSession() {
        val endingSessionId = sessionId
        policyPushJob?.cancel()
        policyPushJob = null
        notificationJob?.cancel()
        notificationJob = null
        lastWardriveSuppressed = null

        gatewayWifi.removeFrameListener(packLinkFrameListener)
        gatewayWifi.removeReplayFrameListener(replayPackLinkFrameListener)
        gatewayWifi.setEventNotifySessionId(0)
        activeIngestor = null

        _sessionIdFlow.value = null
        _wardriveGpsFlow.value = null
        _homeZoneSuppressedFlow.value = false
        _startStateFlow.value = WardriveSessionStartState.Idle
        sessionId = 0

        GatewayLinkService.notifyWardriveActive(this, active = false)

        stopGps()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
        stopSelf()

        val app = application as WardriverApp
        val endingIngestor = ingestor
        ingestor = null
        scope.launch {
            endingIngestor?.shutdown()
            if (endingSessionId > 0) {
                val session = db.sessionDao().get(endingSessionId)
                if (session != null && session.endedUnixMs == null) {
                    db.sessionDao().update(session.copy(endedUnixMs = System.currentTimeMillis()))
                }
            }
            app.appPreferences.setWardriveActive(active = false)
        }
    }

    private suspend fun resumeSession(persistedSessionId: Long) {
        val session = db.sessionDao().get(persistedSessionId)
        if (session == null || session.endedUnixMs != null) {
            (application as WardriverApp).appPreferences.setWardriveActive(active = false)
            failSessionStart("Wardrive session no longer valid")
            return
        }
        beginWardriveSession(insertNewSession = false, existingSessionId = persistedSessionId)
    }

    private suspend fun beginWardriveSession(
        insertNewSession: Boolean,
        existingSessionId: Long? = null,
    ) {
        if (!WardriveClock.isTrusted()) {
            failSessionStart(WardriveStartPreflight.PHONE_TIME_UNSET_MESSAGE)
            return
        }
        // Wake GNSS before SoftAP / catch-up promote so the dashboard is not stuck on idle GPS.
        val locationSource = (application as WardriverApp).appPreferences.locationSource()
        startGps(locationSource)
        val newIngestor: ObservationIngestor =
            when (val promoted = gatewayWifi.promoteCatchUpForWardrive()) {
                null ->
                    if (insertNewSession) {
                        sessionId =
                            db.sessionDao().insert(
                                SessionEntity(
                                    startedUnixMs = WardriveClock.nowUnixMs(),
                                    endedUnixMs = null,
                                    gatewayMac = gatewayWifi.gatewayInfoFlow.value?.gatewayMac.orEmpty(),
                                    sessionKind = SessionKind.WARDRIVE,
                                    geofenceLatMin = null,
                                    geofenceLatMax = null,
                                    geofenceLonMin = null,
                                    geofenceLonMax = null,
                                ),
                            )
                        ObservationIngestor(
                            db = db,
                            sessionId = sessionId,
                            requireInitialFreshGps = true,
                        )
                    } else {
                        sessionId = existingSessionId ?: error("missing wardrive session id")
                        ObservationIngestor(
                            db = db,
                            sessionId = sessionId,
                            requireInitialFreshGps = false,
                        )
                    }
                else -> {
                    sessionId = promoted.first
                    promoted.second
                }
            }

        _sessionIdFlow.value = sessionId
        _startStateFlow.value = WardriveSessionStartState.Running(sessionId)
        openIngestorSession(newIngestor)
        ingestor = newIngestor
        ObservationIngestStats.reset()
        gatewayWifi.setEventNotifySessionId(sessionId)
        activeIngestor = newIngestor
        syncDenylistsToActiveIngestor(applicationContext)
        gatewayWifi.addFrameListener(packLinkFrameListener)
        gatewayWifi.addReplayFrameListener(replayPackLinkFrameListener)
        val app = application as WardriverApp
        app.appPreferences.setScanAllowed(true)
        app.appPreferences.setWardriveActive(
            active = true,
            sessionId = sessionId,
            gatewayMac = gatewayWifi.gatewayInfoFlow.value?.gatewayMac,
        )
        gatewayWifi.connectPreferred()
        GatewayLinkService.notifyWardriveActive(this@WardriveSessionService, active = true)
        watchGatewayForPolicyPush()
        pushPolicySyncIfConnected()
        notificationJob =
            scope.launch {
                while (true) {
                    updateNotification()
                    delay(NOTIFICATION_REFRESH_MS)
                }
            }
    }

    private suspend fun openIngestorSession(ing: ObservationIngestor) {
        try {
            ing.openSession()
        } catch (e: Exception) {
            ObservationIngestStats.recordIngestWarning("enrichment_session_open_failed")
            Log.e(TAG, "enrichment session open failed", e)
        }
    }

    private fun failSessionStart(reason: String) {
        _startStateFlow.value = WardriveSessionStartState.Failed(reason)
        _sessionIdFlow.value = null
        sessionId = 0
        ingestor?.shutdown()
        ingestor = null
        activeIngestor = null
        stopGps()
        _wardriveGpsFlow.value = null
        gatewayWifi.removeFrameListener(packLinkFrameListener)
        gatewayWifi.removeReplayFrameListener(replayPackLinkFrameListener)
        gatewayWifi.setEventNotifySessionId(0)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                stopForeground(STOP_FOREGROUND_REMOVE)
            } else {
                @Suppress("DEPRECATION")
                stopForeground(true)
            }
        } catch (_: Exception) {
        }
        stopSelf()
    }

    private fun watchGatewayForPolicyPush() {
        policyPushJob?.cancel()
        policyPushJob =
            scope.launch {
                var lastConnected = false
                gatewayWifi.connectionState.collect { state ->
                    val connected = state is GatewayWifiRepository.ConnectionState.Connected
                    if (connected && !lastConnected) {
                        pushPolicySyncIfConnected()
                    }
                    lastConnected = connected
                }
            }
    }

    private suspend fun pushPolicySyncIfConnected() {
        if (gatewayWifi.connectionState.value !is GatewayWifiRepository.ConnectionState.Connected) {
            return
        }
        val app = application as WardriverApp
        val snap = app.appPreferences.snapshot()
        val suppressed =
            homeZoneSuppressor.wardriveSuppressed(
                snap,
                lastGpsLat,
                lastGpsLon,
                lastGpsOk,
            )
        lastWardriveSuppressed = suppressed
        val wire =
            PolicySyncAssembler.buildWireBytesWithRoster(
                applicationContext,
                wardriveSuppressed = suppressed,
            ) ?: return
        gatewayWifi.pushTimeSyncThenPolicy(wire)
    }

    private fun maybePushOnHomeZoneChange(suppressed: Boolean) {
        if (lastWardriveSuppressed == suppressed) return
        lastWardriveSuppressed = suppressed
        scope.launch { pushPolicySyncIfConnected() }
    }

    private fun ensureForegroundStarted(subtitle: String): Boolean {
        try {
            val fgType = foregroundServiceTypes()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIFICATION_ID, buildNotification(subtitle), fgType)
            } else {
                @Suppress("DEPRECATION")
                startForeground(NOTIFICATION_ID, buildNotification(subtitle))
            }
            return true
        } catch (e: SecurityException) {
            Log.e(TAG, "startForeground blocked", e)
            failSessionStart("Foreground service blocked: ${e.message ?: "permission"}")
            return false
        }
    }

    private fun foregroundServiceTypes(): Int {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return 0
        var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        if (hasFineLocation()) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
        }
        return types
    }

    private fun hasFineLocation(): Boolean =
        ContextCompat.checkSelfPermission(this, android.Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    private fun startGps(locationSource: LocationSource) {
        stopGps()
        WardriveClock.register(this)
        val lm = getSystemService(LocationManager::class.java)
        val gpsEnabled =
            try {
                lm.isProviderEnabled(LocationManager.GPS_PROVIDER)
            } catch (_: Exception) {
                false
            }
        // Google fused path is opt-in only; the default (GPS_PROVIDER_ONLY) makes zero
        // Play Services location calls. The fused block below is preserved intact.
        if (locationSource == LocationSource.FUSED_OPT_IN) {
            val fused = LocationServices.getFusedLocationProviderClient(this)
            val token = CancellationTokenSource()
            fused.getCurrentLocation(Priority.PRIORITY_HIGH_ACCURACY, token.token)
                .addOnSuccessListener { loc ->
                    if (loc != null && isUsableFix(loc)) {
                        deliverLocationFix(loc, GpsSource.FUSED)
                    }
                }
            val request =
                LocationRequest.Builder(Priority.PRIORITY_HIGH_ACCURACY, GPS_INTERVAL_MS)
                    .setMinUpdateIntervalMillis(GPS_INTERVAL_MS)
                    .setWaitForAccurateLocation(false)
                    .build()
            fused.requestLocationUpdates(request, locationCallback, Looper.getMainLooper())
        }
        // Platform GPS provider registration is unconditional (both modes).
        lm.requestLocationUpdates(
            LocationManager.GPS_PROVIDER,
            GPS_INTERVAL_MS,
            0f,
            gpsProviderListener,
            Looper.getMainLooper(),
        )
        Log.i(
            TAG,
            "startGps mode=${locationSource.name} gpsEnabled=$gpsEnabled interval=${GPS_INTERVAL_MS}ms",
        )
    }

    private fun stopGps() {
        lastLoggedGpsSource = null
        lastGpsLat = null
        lastGpsLon = null
        lastGpsOk = false
        WardriveClock.unregister()
        try {
            LocationServices.getFusedLocationProviderClient(this)
                .removeLocationUpdates(locationCallback)
            getSystemService(LocationManager::class.java)
                .removeUpdates(gpsProviderListener)
        } catch (_: SecurityException) {
        }
    }

    private suspend fun prepareForUpload(): Boolean {
        UploadCoordinator.uploadInProgress = true
        UploadCoordinator.catchUpWasActiveBeforeUpload = gatewayWifi.prepareCatchUpForUpload()

        var resumeWardrive = false
        if (_startStateFlow.value is WardriveSessionStartState.Running) {
            paused = true
            ingestor?.setPaused(true)
            gatewayWifi.removeFrameListener(packLinkFrameListener)
            gatewayWifi.removeReplayFrameListener(replayPackLinkFrameListener)
            activeIngestor = null
            ingestor?.shutdown()
            ingestor = null

            if (sessionId > 0) {
                val session = db.sessionDao().get(sessionId)
                if (session != null && session.endedUnixMs == null) {
                    db.sessionDao().update(session.copy(endedUnixMs = System.currentTimeMillis()))
                }
            }
            stopGps()
            resumeWardrive = true
            UploadCoordinator.resumeWardriveAfterUpload = true
            updateNotification()
        }

        db.sessionDao().closeAllOpenSessions(System.currentTimeMillis())
        return resumeWardrive
    }

    private suspend fun resumeAfterUpload() {
        UploadCoordinator.uploadInProgress = false
        val shouldResumeWardrive = UploadCoordinator.resumeWardriveAfterUpload
        UploadCoordinator.resumeWardriveAfterUpload = false
        val catchUpWasActive = UploadCoordinator.catchUpWasActiveBeforeUpload
        UploadCoordinator.catchUpWasActiveBeforeUpload = false

        val app = application as WardriverApp
        if (shouldResumeWardrive && app.appPreferences.wardriveActive()) {
            beginWardriveSessionAfterUpload()
        } else if (catchUpWasActive && !app.appPreferences.wardriveActive()) {
            gatewayWifi.resumeCatchUpAfterUploadIfNeeded()
        }
    }

    private suspend fun beginWardriveSessionAfterUpload() {
        if (!WardriveClock.isTrusted()) {
            failSessionStart(WardriveStartPreflight.PHONE_TIME_UNSET_MESSAGE)
            return
        }
        val locationSource = (application as WardriverApp).appPreferences.locationSource()
        startGps(locationSource)
        sessionId =
            db.sessionDao().insert(
                SessionEntity(
                    startedUnixMs = WardriveClock.nowUnixMs(),
                    endedUnixMs = null,
                    gatewayMac = gatewayWifi.gatewayInfoFlow.value?.gatewayMac.orEmpty(),
                    sessionKind = SessionKind.WARDRIVE,
                    geofenceLatMin = null,
                    geofenceLatMax = null,
                    geofenceLonMin = null,
                    geofenceLonMax = null,
                ),
            )
        val newIngestor =
            ObservationIngestor(
                db = db,
                sessionId = sessionId,
                requireInitialFreshGps = false,
            )
        _sessionIdFlow.value = sessionId
        _startStateFlow.value = WardriveSessionStartState.Running(sessionId)
        openIngestorSession(newIngestor)
        ingestor = newIngestor
        ObservationIngestStats.reset()
        gatewayWifi.setEventNotifySessionId(sessionId)
        activeIngestor = newIngestor
        syncDenylistsToActiveIngestor(applicationContext)
        gatewayWifi.addFrameListener(packLinkFrameListener)
        gatewayWifi.addReplayFrameListener(replayPackLinkFrameListener)
        val app = application as WardriverApp
        app.appPreferences.setScanAllowed(true)
        app.appPreferences.setWardriveActive(
            active = true,
            sessionId = sessionId,
            gatewayMac = gatewayWifi.gatewayInfoFlow.value?.gatewayMac,
        )
        paused = false
        ingestor?.setPaused(false)
        gatewayWifi.connectPreferred()
        GatewayLinkService.notifyWardriveActive(this@WardriveSessionService, active = true)
        watchGatewayForPolicyPush()
        pushPolicySyncIfConnected()
        notificationJob?.cancel()
        notificationJob =
            scope.launch {
                while (true) {
                    updateNotification()
                    delay(NOTIFICATION_REFRESH_MS)
                }
            }
        updateNotification()
    }

    private fun deliverLocationFix(loc: Location, source: GpsSource) {
        if (UploadCoordinator.uploadInProgress) {
            return
        }
        // Always refresh dashboard display (including coarse hunting).
        _wardriveGpsFlow.value = loc
        if (!isUsableFix(loc)) {
            return
        }
        maybeLogGpsSource(source, loc.accuracy)
        applyUsableLocationFix(loc)
    }

    private fun applyUsableLocationFix(loc: Location) {
        val accuracy = loc.accuracy
        lastGpsLat = loc.latitude
        lastGpsLon = loc.longitude
        lastGpsOk = accuracy > 0f && accuracy <= ObservationIngestor.ROUTE_MAX_ACCURACY_M
        _wardriveGpsFlow.value = loc
        ingestor?.onGpsFix(
            loc.time,
            loc.latitude,
            loc.longitude,
            accuracy,
            loc.altitude,
            isFresh = GpsFreshness.isFresh(loc),
        )
        gatewayWifi.sendTimeSyncIfConnected()

        scope.launch {
            val app = application as WardriverApp
            val snap = app.appPreferences.snapshot()
            val suppressed =
                homeZoneSuppressor.wardriveSuppressed(
                    snap,
                    loc.latitude,
                    loc.longitude,
                    lastGpsOk,
                )
            _homeZoneSuppressedFlow.value = suppressed
            maybePushOnHomeZoneChange(suppressed)
            ingestor?.setInsideHomeGeofence(suppressed)
        }

        geofenceMonitor?.let { monitor ->
            when (monitor.update(loc.latitude, loc.longitude)) {
                GeofenceMonitor.GeofenceEvent.Entered -> onGeofenceEnter()
                GeofenceMonitor.GeofenceEvent.Left -> onGeofenceLeave()
                else -> Unit
            }
        }

        if (lastGpsOk && GpsTimeline.isValidFixTime(loc.time)) {
            scope.launch {
                db.routeDao().insert(
                    RoutePointEntity(
                        sessionId = sessionId,
                        unixMs = loc.time,
                        lat = loc.latitude,
                        lon = loc.longitude,
                        accuracyM = accuracy,
                    ),
                )
            }
            maybeIngestCellTowers(loc)
        }
    }

    private fun isUsableFix(loc: Location): Boolean {
        val accuracy = loc.accuracy
        return accuracy > 0f &&
            accuracy <= ObservationIngestor.ROUTE_MAX_ACCURACY_M &&
            GpsTimeline.isValidFixTime(loc.time)
    }

    private fun maybeLogGpsSource(source: GpsSource, accuracyM: Float) {
        if (source == lastLoggedGpsSource) return
        lastLoggedGpsSource = source
        Log.i(TAG, "gps source=${source.logLabel} acc=${accuracyM}m")
    }

    private enum class GpsSource(val logLabel: String) {
        FUSED("fused"),
        GPS_PROVIDER("gps_provider"),
    }

    private fun maybeIngestCellTowers(loc: Location) {
        if (paused) return
        scope.launch {
            val app = application as WardriverApp
            if (!app.appPreferences.isCellTowersEnabled()) return@launch
            if (!cellCollector.hasCellPermissions()) return@launch
            val now = loc.time
            if (now - lastCellScanUnixMs < CELL_SCAN_INTERVAL_MS) return@launch
            lastCellScanUnixMs = now
            val cells = cellCollector.collect(operatorName = null)
            val active = ingestor ?: return@launch
            active.ingestCellTowers(
                cells = cells,
                observedUnixMs = loc.time,
                lat = loc.latitude,
                lon = loc.longitude,
                accuracyM = loc.accuracy,
                altitudeM = loc.altitude.takeIf { loc.hasAltitude() },
            )
        }
    }

    private fun onGeofenceEnter() {
        scope.launch {
            GeofenceTransitionNotifier.notify(
                this@WardriveSessionService,
                GeofenceTransitionNotifier.Transition.Enter,
            )
        }
    }

    private fun onGeofenceLeave() {
        scope.launch {
            GeofenceTransitionNotifier.notify(
                this@WardriveSessionService,
                GeofenceTransitionNotifier.Transition.Leave,
            )
        }
    }

    /** Apply or clear geofence bounds; persists session row and pushes policy_sync. */
    private fun applyGeofence(
        latMin: Double = 0.0,
        latMax: Double = 0.0,
        lonMin: Double = 0.0,
        lonMax: Double = 0.0,
        enabled: Boolean = false,
        clear: Boolean = false,
    ) {
        if (clear) {
            geofenceMonitor = null
        } else {
            geofenceMonitor = GeofenceMonitor(latMin, latMax, lonMin, lonMax, enabled)
        }
        scope.launch {
            val session = db.sessionDao().get(sessionId) ?: return@launch
            if (clear) {
                db.sessionDao().update(
                    session.copy(
                        geofenceLatMin = null,
                        geofenceLatMax = null,
                        geofenceLonMin = null,
                        geofenceLonMax = null,
                    ),
                )
            } else {
                db.sessionDao().update(
                    session.copy(
                        geofenceLatMin = latMin,
                        geofenceLatMax = latMax,
                        geofenceLonMin = lonMin,
                        geofenceLonMax = lonMax,
                    ),
                )
                val app = application as WardriverApp
                app.appPreferences.saveGeofence(
                    GeofenceBounds(
                        latMin = latMin,
                        latMax = latMax,
                        lonMin = lonMin,
                        lonMax = lonMax,
                    ),
                )
            }
            if (clear) {
                (application as WardriverApp).appPreferences.clearGeofence()
            }
            pushPolicySyncIfConnected()
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
        val pauseIntent =
            PendingIntent.getService(
                this,
                1,
                Intent(this, WardriveSessionService::class.java).setAction(ACTION_PAUSE),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val resumeIntent =
            PendingIntent.getService(
                this,
                2,
                Intent(this, WardriveSessionService::class.java).setAction(ACTION_RESUME),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val uploadIntent =
            PendingIntent.getService(
                this,
                3,
                Intent(this, WardriveSessionService::class.java).setAction(ACTION_UPLOAD),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val gatewayConnected =
            gatewayWifi.connectionState.value is GatewayWifiRepository.ConnectionState.Connected
        val gatewayLabel = if (gatewayConnected) "Gateway on" else "Gateway off"
        val pauseLabel = if (paused) "Paused" else "Running"
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("wolfPACK wardrive")
            .setContentText("$subtitle - $gatewayLabel - $pauseLabel")
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(0, "Pause", pauseIntent)
            .addAction(0, "Resume", resumeIntent)
            .addAction(0, "Upload", uploadIntent)
            .build()
    }

    private suspend fun updateNotification() {
        val count = db.observationDao().countForSession(sessionId)
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.notify(
            NOTIFICATION_ID,
            buildNotification(
                "Obs $count - revisit ${NativeCore.revisitRadiusMOrDefault()} m",
            ),
        )
    }

    private fun createChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Wardrive", NotificationManager.IMPORTANCE_LOW),
        )
        AlertChannels.ensureCreated(this)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        policyPushJob?.cancel()
        notificationJob?.cancel()
        stopGps()
        WardriveClock.reset()
        gatewayWifi.removeFrameListener(packLinkFrameListener)
        gatewayWifi.removeReplayFrameListener(replayPackLinkFrameListener)
        ingestor?.shutdown()
        ingestor = null
        activeIngestor = null
        val endingSessionId = sessionId
        if (endingSessionId > 0) {
            runBlocking {
                val session = db.sessionDao().get(endingSessionId)
                if (session != null && session.endedUnixMs == null) {
                    db.sessionDao().update(session.copy(endedUnixMs = System.currentTimeMillis()))
                }
            }
        }
        _sessionIdFlow.value = null
        _wardriveGpsFlow.value = null
        _homeZoneSuppressedFlow.value = false
        if (_startStateFlow.value is WardriveSessionStartState.Running ||
            _startStateFlow.value is WardriveSessionStartState.Starting
        ) {
            _startStateFlow.value = WardriveSessionStartState.Idle
        }
        super.onDestroy()
    }

    companion object {
        private const val TAG = "WardriveSessionService"

        private val _startStateFlow = MutableStateFlow<WardriveSessionStartState>(WardriveSessionStartState.Idle)
        val startStateFlow: StateFlow<WardriveSessionStartState> = _startStateFlow.asStateFlow()

        private val _sessionIdFlow = MutableStateFlow<Long?>(null)
        val sessionIdFlow: StateFlow<Long?> = _sessionIdFlow.asStateFlow()

        private val _wardriveGpsFlow = MutableStateFlow<Location?>(null)
        /** Latest wardrive GPS fix (session service); dashboard prefers this over one-shot fused. */
        val wardriveGpsFlow: StateFlow<Location?> = _wardriveGpsFlow.asStateFlow()

        private val _homeZoneSuppressedFlow = MutableStateFlow(false)
        /** Home-zone latch for CSV gate (matches ingest `wardrive_suppressed`). */
        val homeZoneSuppressedFlow: StateFlow<Boolean> = _homeZoneSuppressedFlow.asStateFlow()

        @Volatile
        private var activeIngestor: ObservationIngestor? = null

        /** Push WiGLE denylist prefs into the running wardrive ingestor (no-op when idle). */
        suspend fun syncDenylistsToActiveIngestor(context: Context) {
            val ing = activeIngestor ?: return
            val app = context.wardriverApp()
            val snap = app.appPreferences.snapshot()
            val gatewayMac = app.gatewayWifiRepository.gatewayInfoFlow.value?.gatewayMac
            ing.setDenylists(
                DenylistBssids.mergedManualPlusGateway(
                    AppPreferences.parseLineList(snap.denylistBssidsText),
                    gatewayMac,
                ),
                AppPreferences.parseLineList(snap.denylistSsidsText),
            )
        }

        const val CHANNEL_ID = "wardrive"
        const val NOTIFICATION_ID = 42
        const val ACTION_PAUSE = "net.wardriver.mobile.PAUSE"
        const val ACTION_RESUME = "net.wardriver.mobile.RESUME"
        const val ACTION_UPLOAD = "net.wardriver.mobile.UPLOAD"
        const val ACTION_CONFIGURE_GEOFENCE = "net.wardriver.mobile.CONFIGURE_GEOFENCE"
        const val ACTION_CLEAR_GEOFENCE = "net.wardriver.mobile.CLEAR_GEOFENCE"
        const val ACTION_STOP = "net.wardriver.mobile.STOP"
        const val ACTION_START = "net.wardriver.mobile.START"
        const val ACTION_PREPARE_FOR_UPLOAD = "net.wardriver.mobile.PREPARE_FOR_UPLOAD"
        const val ACTION_RESUME_AFTER_UPLOAD = "net.wardriver.mobile.RESUME_AFTER_UPLOAD"

        const val EXTRA_LAT_MIN = "lat_min"
        const val EXTRA_LAT_MAX = "lat_max"
        const val EXTRA_LON_MIN = "lon_min"
        const val EXTRA_LON_MAX = "lon_max"

        /** GPS sampling interval aligned with control-node timeline (500 ms). */
        private const val GPS_INTERVAL_MS = 500L

        /** Minimum interval between `getAllCellInfo()` scans during wardrive. */
        private const val CELL_SCAN_INTERVAL_MS = 20_000L
        private const val NOTIFICATION_REFRESH_MS = 15_000L

        /** Restore session id flow after process death (before UI subscribes). */
        fun restoreSessionId(id: Long) {
            _sessionIdFlow.value = id
            _startStateFlow.value = WardriveSessionStartState.Running(id)
        }

        /** Start foreground wardrive; returns false when OS blocks background FGS start. */
        fun start(context: Context): Boolean {
            val current = _startStateFlow.value
            if (current is WardriveSessionStartState.Starting || current is WardriveSessionStartState.Running) {
                return true
            }
            return try {
                context.startForegroundService(
                    Intent(context, WardriveSessionService::class.java).apply {
                        action = ACTION_START
                    },
                )
                true
            } catch (e: Exception) {
                val reason =
                    if (Build.VERSION.SDK_INT >= 31 && e is ForegroundServiceStartNotAllowedException) {
                        "Wardrive blocked: open the app and try again"
                    } else {
                        "Wardrive start failed: ${e.message ?: e.javaClass.simpleName}"
                    }
                _startStateFlow.value = WardriveSessionStartState.Failed(reason)
                false
            }
        }

        fun clearFailedStart() {
            if (_startStateFlow.value is WardriveSessionStartState.Failed) {
                _startStateFlow.value = WardriveSessionStartState.Idle
            }
        }

        /** Stop active wardrive session and foreground service. */
        fun stop(context: Context) {
            context.startService(
                Intent(context, WardriveSessionService::class.java).setAction(ACTION_STOP),
            )
        }

        /** Wait until [sessionIdFlow] clears after [stop] (or already idle). */
        suspend fun awaitSessionEnded(timeoutMs: Long): Boolean {
            if (sessionIdFlow.value == null) return true
            return withTimeoutOrNull(timeoutMs) {
                sessionIdFlow.first { it == null }
                true
            } ?: false
        }

        /** Map UI → running session: persist bounds, arm monitor, push policy_sync. */
        fun configureGeofence(
            context: Context,
            latMin: Double,
            latMax: Double,
            lonMin: Double,
            lonMax: Double,
        ) {
            val intent =
                Intent(context, WardriveSessionService::class.java).apply {
                    action = ACTION_CONFIGURE_GEOFENCE
                    putExtra(EXTRA_LAT_MIN, latMin)
                    putExtra(EXTRA_LAT_MAX, latMax)
                    putExtra(EXTRA_LON_MIN, lonMin)
                    putExtra(EXTRA_LON_MAX, lonMax)
                }
            context.startService(intent)
        }

        /** Map UI → running session: clear bounds, disarm monitor, push policy_sync. */
        fun clearGeofence(context: Context) {
            context.startService(
                Intent(context, WardriveSessionService::class.java).setAction(ACTION_CLEAR_GEOFENCE),
            )
        }
    }
}
