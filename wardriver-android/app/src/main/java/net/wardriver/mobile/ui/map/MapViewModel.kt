package net.wardriver.mobile.ui.map

import android.content.Context
import android.location.Location
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.sample
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.MapStore
import net.wardriver.mobile.data.OneShotLocation
import net.wardriver.mobile.data.RoutePoint
import net.wardriver.mobile.db.SessionWithCount
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.wardriverApp
import org.osmdroid.util.GeoPoint

@OptIn(ExperimentalCoroutinesApi::class, FlowPreview::class)
class MapViewModel(
    private val db: WardriverDatabase,
    private val appContext: Context,
) : ViewModel() {
    private val appPreferences: AppPreferences = appContext.wardriverApp().appPreferences
    private val gatewayWifi = appContext.wardriverApp().gatewayWifiRepository
    private val mapStore =
        MapStore(
            db.sessionDao(),
            db.routeDao(),
            db.detectionDao(),
            db.sessionMapMarkerDao(),
            db.probeObservationDao(),
            db.networkSummaryDao(),
        )

    val sessionIdState: StateFlow<Long?> =
        WardriveSessionService.sessionIdFlow.stateIn(
            viewModelScope,
            SharingStarted.WhileSubscribed(5_000),
            null,
        )

    private val _state = MutableStateFlow(MapUiState())
    val state: StateFlow<MapUiState> = _state.asStateFlow()

    private var gpsUsable = false
    private var apCentroid: GeoPoint? = null
    private var homeZoneCenter: GeoPoint? = null

    private var rawApMarkers: List<ApMarker> = emptyList()
    private var rawAlertPins: List<AlertPin> = emptyList()
    private var rawRouteEntities: List<RoutePoint> = emptyList()
    private var routeGeoPoints: MutableList<GeoPoint> = mutableListOf()
    private var lastRoutePointId: Long = -1L
    private var viewport: MapViewport? = null
    private val viewportFlow = MutableStateFlow<MapViewport?>(null)
    private var mapActive = false
    private var cachedDisplaySessionId: Long? = null
    private var clusterRebuildJob: Job? = null
    private var coverageJob: Job? = null
    private var sessionBindJob: Job? = null
    private var markerBindJob: Job? = null
    private var driverLockBeforeRemoteIdFocus = false

    init {
        observeHomeZoneCenter()
        observeMapPreferences()
        observeWardriveGps()
        observeActiveSession()
    }

    fun setMapActive(active: Boolean) {
        if (mapActive == active) return
        mapActive = active
        if (!active) {
            sessionBindJob?.cancel()
            markerBindJob?.cancel()
            clusterRebuildJob?.cancel()
            coverageJob?.cancel()
            sessionBindJob = null
            markerBindJob = null
        } else {
            val sessionId = cachedDisplaySessionId ?: _state.value.displaySessionId
            bindMapData(sessionId)
            refreshGps()
            viewport?.let { viewportFlow.value = it }
        }
    }

    fun selectCluster(cluster: ApCluster?) {
        _state.update { it.copy(selectedCluster = cluster) }
    }

    fun clearSelectedCluster() {
        selectCluster(null)
    }

    fun selectAlertPin(pin: AlertPin?) {
        val wasRemoteIdFocus = _state.value.selectedAlertPin?.kind == AlertKind.REMOTE_ID
        if (pin?.kind == AlertKind.REMOTE_ID) {
            if (!wasRemoteIdFocus) {
                driverLockBeforeRemoteIdFocus = _state.value.driverLockEnabled
                if (driverLockBeforeRemoteIdFocus) {
                    setDriverLock(false)
                }
            }
            val operatorPoint = remoteIdOperatorPoint(pin)
            _state.update {
                it.copy(
                    selectedAlertPin = pin,
                    remoteIdOperatorPoint = operatorPoint,
                    remoteIdFocusNonce = it.remoteIdFocusNonce + 1,
                )
            }
            publishRemoteIdFocusedLayers(pin)
        } else {
            if (wasRemoteIdFocus) {
                restoreDriverLockAfterRemoteIdFocus()
            }
            _state.update {
                it.copy(
                    selectedAlertPin = pin,
                    remoteIdOperatorPoint = null,
                )
            }
            rebuildVisibleClusters()
            applyAlertFilter()
        }
    }

    fun clearSelectedAlertPin() {
        val wasRemoteIdFocus = _state.value.selectedAlertPin?.kind == AlertKind.REMOTE_ID
        _state.update {
            it.copy(
                selectedAlertPin = null,
                remoteIdOperatorPoint = null,
            )
        }
        if (wasRemoteIdFocus) {
            restoreDriverLockAfterRemoteIdFocus()
            rebuildVisibleClusters()
            applyAlertFilter()
        }
    }

    private fun restoreDriverLockAfterRemoteIdFocus() {
        if (driverLockBeforeRemoteIdFocus) {
            setDriverLock(true)
        }
        driverLockBeforeRemoteIdFocus = false
    }

    private fun publishRemoteIdFocusedLayers(pin: AlertPin) {
        _state.update {
            it.copy(
                accessPointClusters = emptyList(),
                alertPins = listOf(pin),
            )
        }
    }

    fun onMapVisible() {
        refreshGps()
    }

    fun recenter() {
        refreshGps()
        _state.update { it.copy(recenterNonce = it.recenterNonce + 1) }
        publishCenterTarget()
    }

    fun toggleDriverLock() {
        if (sessionIdState.value == null) return
        setDriverLock(!_state.value.driverLockEnabled)
    }

    fun setDriverLock(enabled: Boolean) {
        if (sessionIdState.value == null) return
        _state.update {
            it.copy(
                driverLockEnabled = enabled,
                recenterNonce = if (enabled) it.recenterNonce + 1 else it.recenterNonce,
            )
        }
        if (enabled) {
            publishCenterTarget()
        }
    }

    fun refreshGps() {
        viewModelScope.launch {
            val loc = OneShotLocation.resolve(appContext)
            if (loc != null && isGpsCenterUsable(loc)) {
                applyGpsLocation(loc)
            } else {
                applyGpsLocation(null)
            }
        }
    }

    private fun applyGpsLocation(loc: Location?) {
        if (!mapActive) return
        if (loc == null) {
            gpsUsable = false
            _state.update { it.copy(userLocation = null) }
        } else {
            gpsUsable = true
            _state.update { it.copy(userLocation = GeoPoint(loc.latitude, loc.longitude)) }
        }
        publishCenterTarget()
    }

    private fun isGpsCenterUsable(loc: Location): Boolean =
        !loc.hasAccuracy() || loc.accuracy < GPS_CENTER_MAX_ACCURACY_M

    private fun observeHomeZoneCenter() {
        viewModelScope.launch {
            combine(
                appPreferences.homeZoneEnabledFlow,
                appPreferences.homeZoneLatFlow,
                appPreferences.homeZoneLonFlow,
            ) { enabled, lat, lon ->
                if (enabled && lat != null && lon != null) GeoPoint(lat, lon) else null
            }.collect { center ->
                homeZoneCenter = center
                if (mapActive) {
                    publishCenterTarget()
                }
            }
        }
    }

    private fun observeMapPreferences() {
        viewModelScope.launch {
            combine(
                appPreferences.mapDataScopeFlow,
                appPreferences.mapLayerFiltersFlow,
                appPreferences.mapCoverageShadingEnabledFlow,
                appPreferences.mapBasemapFlow,
                appPreferences.mapLayerPanelExpandedFlow,
            ) { scope, filters, coverageShading, basemap, panelExpanded ->
                Quad(scope, filters, coverageShading to basemap, panelExpanded)
            }.collect { (scope, filters, coverageBasemap, panelExpanded) ->
                val (coverageShading, basemap) = coverageBasemap
                _state.update {
                    it.copy(
                        dataScope = scope,
                        layerFilters = filters,
                        coverageShadingEnabled = coverageShading,
                        mapBasemap = basemap,
                        layerPanelExpanded = panelExpanded,
                    )
                }
                if (mapActive) {
                    rebuildVisibleClusters()
                    applyAlertFilter()
                    _state.value.selectedAlertPin
                        ?.takeIf { it.kind == AlertKind.REMOTE_ID }
                        ?.let { publishRemoteIdFocusedLayers(it) }
                    scheduleCoverage()
                }
            }
        }
    }

    private data class Quad<A, B, C, D>(val first: A, val second: B, val third: C, val fourth: D)

    private fun observeWardriveGps() {
        viewModelScope.launch {
            combine(sessionIdState, WardriveSessionService.wardriveGpsFlow) { sessionId, wardriveLoc ->
                sessionId to wardriveLoc
            }.collect { (sessionId, wardriveLoc) ->
                if (!mapActive) return@collect
                if (sessionId != null && wardriveLoc != null) {
                    applyGpsLocation(wardriveLoc)
                }
            }
        }
    }

    private fun observeActiveSession() {
        viewModelScope.launch {
            sessionIdState.collect { activeId ->
                val latestId = mapStore.latestSessionId()
                if (activeId != null) {
                    // Active wardrive: clear user-selected session so active takes priority
                    _state.update {
                        it.copy(
                            displaySessionId = activeId,
                            selectedSessionId = null,
                            hasActiveWardrive = true,
                            driverLockEnabled = it.driverLockEnabled,
                        )
                    }
                } else {
                    // No active wardrive: use selected or latest, unless in AllDevices mode
                    val displayId = if (_state.value.dataScope == MapDataScope.AllDevices) {
                        null
                    } else {
                        resolveDisplaySessionId(
                            _state.value.selectedSessionId,
                            latestId,
                        )
                    }
                    _state.update {
                        it.copy(
                            displaySessionId = displayId,
                            hasActiveWardrive = false,
                            driverLockEnabled = false,
                        )
                    }
                }
                cachedDisplaySessionId = _state.value.displaySessionId
                if (mapActive) {
                    bindMapData(_state.value.displaySessionId)
                }
            }
        }
    }

    private fun bindMapData(displaySessionId: Long?) {
        sessionBindJob?.cancel()
        markerBindJob?.cancel()
        resetSessionRawData()
        if (!mapActive) return
        sessionBindJob =
            viewModelScope.launch {
                if (displaySessionId != null) {
                    observeSessionScoped(displaySessionId)
                } else {
                    observePrefsOnlyScoped()
                }
            }
        markerBindJob =
            viewModelScope.launch {
                observeMarkerPipeline(displaySessionId)
            }
    }

    private fun resetSessionRawData() {
        rawApMarkers = emptyList()
        rawAlertPins = emptyList()
        rawRouteEntities = emptyList()
        routeGeoPoints = mutableListOf()
        lastRoutePointId = -1L
        apCentroid = null
        _state.update {
            it.copy(
                routePoints = emptyList(),
                rawRouteCount = 0,
                accessPointClusters = emptyList(),
                alertPins = emptyList(),
                coverageUnvisited = emptyList(),
                coverageFraction = 0.0,
                loadingCount = null,
            )
        }
    }

    private fun publishCenterTarget() {
        val current = _state.value
        val target =
            resolveCenterTarget(
                userLocation = current.userLocation,
                gpsUsable = gpsUsable,
                routePoints = current.routePoints,
                apCentroid = apCentroid,
                homeZoneCenter = homeZoneCenter,
                geofence = current.geofence,
            )
        _state.update { it.copy(mapCenterTarget = target) }
    }

    private suspend fun observeSessionScoped(sessionId: Long) {
        coroutineScope {
            launch {
                mapStore.sessionGeofenceFlow(sessionId).collect { geofence ->
                    _state.update { it.copy(geofence = geofence) }
                    if (mapActive) {
                        scheduleCoverage()
                    }
                }
            }

            launch {
                val initial = mapStore.initialRoutePoints(sessionId)
                if (!mapActive) return@launch
                rawRouteEntities = initial
                lastRoutePointId = initial.lastOrNull()?.id ?: -1L
                routeGeoPoints = initial.map { GeoPoint(it.lat, it.lon) }.toMutableList()
                publishRouteState()

                mapStore
                    .latestRoutePointFlow(sessionId)
                    .collect { latest ->
                        if (!mapActive || latest == null) return@collect
                        if (latest.id <= lastRoutePointId) return@collect
                        lastRoutePointId = latest.id
                        rawRouteEntities = rawRouteEntities + latest
                        routeGeoPoints.add(GeoPoint(latest.lat, latest.lon))
                        publishRouteState()
                    }
            }

            launch {
                observeScopedDetections(sessionId)
            }
        }
    }

    private fun publishRouteState() {
        val simplified = MapGeometry.simplifyRoute(routeGeoPoints.toList())
        _state.update {
            it.copy(routePoints = simplified, rawRouteCount = rawRouteEntities.size)
        }
        publishCenterTarget()
        scheduleCoverage()
    }

    private suspend fun observeMarkerPipeline(displaySessionId: Long?) {
        combine(
            viewportFlow.debounce(VIEWPORT_DEBOUNCE_MS),
            appPreferences.mapDataScopeFlow,
        ) { vp, scope -> vp to scope }
            .flatMapLatest { (vp, scope) ->
                if (!mapActive || vp == null) {
                    flowOf(emptyList<ApMarker>())
                } else {
                    when (scope) {
                        MapDataScope.Session -> {
                            val sessionId = displaySessionId
                            if (sessionId == null) {
                                flowOf(emptyList<ApMarker>()).debounce(MAP_UPDATE_DEBOUNCE_MS)
                            } else {
                                mapStore.apMarkersForSession(sessionId, vp)
                                    .debounce(MAP_UPDATE_DEBOUNCE_MS)
                            }
                        }
                        MapDataScope.AllDevices ->
                            mapStore.apMarkersAllDevices(vp)
                                .debounce(MAP_UPDATE_DEBOUNCE_MS_ALL)
                    }
                }
            }
            .collect { markers ->
                if (!mapActive) return@collect
                rawApMarkers = markers
                apCentroid = centroidFromApMarkers(markers)
                _state.update { it.copy(isLoading = false) }
                rebuildVisibleClusters()
                publishCenterTarget()
            }
    }

    private suspend fun observeScopedDetections(sessionId: Long?) {
        appPreferences.mapDataScopeFlow
            .flatMapLatest { scope ->
                when (scope) {
                    MapDataScope.Session ->
                        if (sessionId != null) {
                            mapStore.alertPinsForSession(sessionId)
                        } else {
                            flowOf(emptyList<AlertPin>())
                        }
                    MapDataScope.AllDevices -> {
                        _state.update { it.copy(isLoading = true) }
                        viewportFlow
                            .sample(VIEWPORT_SAMPLE_MS_ALL)
                            .flatMapLatest { vp ->
                                if (vp == null) {
                                    flowOf(emptyList<AlertPin>())
                                } else {
                                    mapStore.alertPinsInViewport(vp)
                                }
                            }
                    }
                }
            }
            .debounce(MAP_UPDATE_DEBOUNCE_MS)
            .collect { alertPins ->
                if (!mapActive) return@collect
                rawAlertPins = alertPins
                _state.update {
                    it.copy(
                        isLoading = false,
                        loadingCount = if (_state.value.dataScope == MapDataScope.AllDevices) {
                            rawApMarkers.size + alertPins.size
                        } else null,
                    )
                }
                rebuildAlertPins()
            }
    }

    private suspend fun observePrefsOnlyScoped() {
        coroutineScope {
            launch {
                appPreferences.geofenceFlow.collect { prefsGeofence ->
                    _state.update { it.copy(geofence = prefsGeofence) }
                    if (mapActive) {
                        scheduleCoverage()
                    }
                }
            }

            launch {
                observeScopedDetections(sessionId = null)
            }
        }
    }

    fun setZoomLevel(zoom: Double) {
        if (kotlin.math.abs(_state.value.zoomLevel - zoom) < 0.05) return
        _state.update { it.copy(zoomLevel = zoom) }
        scheduleClusterRebuild()
    }

    fun onViewportChanged(viewport: MapViewport) {
        this.viewport = viewport
        viewportFlow.value = viewport
    }

    fun setDataScope(scope: MapDataScope) {
        // Intentionally do not persist — always reset to Session on app restart
        _state.update { it.copy(dataScope = scope) }
        // If in AllDevices mode, rebind with null displaySessionId
        if (scope == MapDataScope.AllDevices) {
            cachedDisplaySessionId = null
            _state.update { it.copy(displaySessionId = null, isLoading = true) }
            if (mapActive) bindMapData(null)
        }
    }

    fun setLayerFilters(filters: MapLayerFilters) {
        viewModelScope.launch { appPreferences.setMapLayerFilters(filters) }
    }

    fun setCoverageShadingEnabled(enabled: Boolean) {
        viewModelScope.launch { appPreferences.setMapCoverageShadingEnabled(enabled) }
    }

    // --- Layer panel (collapsed behind FAB) ---

    fun setLayerPanelExpanded(expanded: Boolean) {
        viewModelScope.launch { appPreferences.setMapLayerPanelExpanded(expanded) }
    }

    // --- Geofence editor visibility toggle ---

    fun toggleGeofenceEditorVisibility() {
        _state.update { it.copy(geofenceEditorVisible = !it.geofenceEditorVisible) }
    }

    // --- Session picker ---

    private val _sessions = MutableStateFlow<List<SessionWithCount>>(emptyList())
    val sessions: StateFlow<List<SessionWithCount>> = _sessions.asStateFlow()

    fun loadSessions() {
        viewModelScope.launch {
            _sessions.value = mapStore.allSessionsWithCounts()
        }
    }

    fun setSelectedSessionId(id: Long?) {
        _state.update { it.copy(selectedSessionId = id) }
        if (id != null) {
            viewModelScope.launch {
                val latestId = mapStore.latestSessionId()
                val displayId = resolveDisplaySessionId(id, latestId)
                if (displayId != null) {
                    cachedDisplaySessionId = displayId
                    _state.update { it.copy(displaySessionId = displayId) }
                    if (mapActive) bindMapData(displayId)
                }
            }
        }
    }

    // --- Text search ---

    fun setSearchQuery(query: String) {
        _state.update { it.copy(searchQuery = query.trim()) }
        updateSearchResultCount()
    }

    private fun updateSearchResultCount() {
        val query = _state.value.searchQuery
        if (query.isEmpty()) {
            _state.update { it.copy(searchResultCount = _state.value.accessPointClusters.size) }
            return
        }
        val q = query.lowercase()
        val count = _state.value.accessPointClusters.count { cluster ->
            cluster.members.any { ap ->
                ap.bssid.lowercase().contains(q) ||
                    ap.ssid.lowercase().contains(q)
            }
        }
        _state.update { it.copy(searchResultCount = count) }
    }

    private fun scheduleClusterRebuild() {
        clusterRebuildJob?.cancel()
        clusterRebuildJob =
            viewModelScope.launch {
                delay(CLUSTER_REBUILD_DEBOUNCE_MS)
                rebuildVisibleClusters()
            }
    }

    private fun rebuildVisibleClusters() {
        if (!mapActive) return
        clusterRebuildJob?.cancel()
        val debounceMs = if (_state.value.dataScope == MapDataScope.AllDevices)
            CLUSTER_REBUILD_DEBOUNCE_MS_ALL else CLUSTER_REBUILD_DEBOUNCE_MS
        clusterRebuildJob =
            viewModelScope.launch {
                delay(debounceMs)
                if (!mapActive) return@launch
                val aps = _state.value.layerFilters.filterApMarkers(rawApMarkers)
                val zoom = _state.value.zoomLevel
                val clusters =
                    withContext(Dispatchers.Default) {
                        MapGeometry.clusterMapMarkers(aps, zoom)
                    }
                if (mapActive) {
                    _state.update { it.copy(accessPointClusters = remoteIdFocusedClusters(clusters, it.selectedAlertPin)) }
                }
            }
    }

    private fun rebuildAlertPins() {
        applyAlertFilter()
    }

    private fun applyAlertFilter() {
        val selected = _state.value.selectedAlertPin
        if (selected?.kind == AlertKind.REMOTE_ID) {
            _state.update { it.copy(alertPins = listOf(selected)) }
            return
        }
        val filtered = _state.value.layerFilters.filterAlertPins(rawAlertPins)
        _state.update { it.copy(alertPins = remoteIdFocusedAlertPins(filtered, selected)) }
    }

    private fun scheduleCoverage() {
        if (!mapActive) return
        coverageJob?.cancel()
        coverageJob =
            viewModelScope.launch {
                delay(COVERAGE_DEBOUNCE_MS)
                if (!mapActive) return@launch
                val geofence = _state.value.geofence
                if (geofence == null) {
                    _state.update {
                        it.copy(coverageUnvisited = emptyList(), coverageFraction = 0.0)
                    }
                    return@launch
                }
                val coverage = computeCoverage(geofence, rawRouteEntities)
                _state.update {
                    it.copy(
                        coverageFraction = coverage.fraction,
                        coverageUnvisited =
                            if (_state.value.coverageShadingEnabled) {
                                coverage.unvisited
                            } else {
                                emptyList()
                            },
                    )
                }
            }
    }

    private data class CoverageResult(
        val fraction: Double,
        val unvisited: List<GridCell>,
    )

    private fun computeCoverage(
        geofence: GeofenceBounds,
        route: List<RoutePoint>,
    ): CoverageResult {
        val lats = DoubleArray(route.size)
        val lons = DoubleArray(route.size)
        for (i in route.indices) {
            lats[i] = route[i].lat
            lons[i] = route[i].lon
        }
        val native =
            NativeCore.coverageGridFromArrays(
                latMin = geofence.latMin,
                latMax = geofence.latMax,
                lonMin = geofence.lonMin,
                lonMax = geofence.lonMax,
                rows = COVERAGE_ROWS,
                cols = COVERAGE_COLS,
                routeLats = lats,
                routeLons = lons,
            )
        if (native != null) {
            val cells = mutableListOf<GridCell>()
            for (i in native.unvisitedRows.indices) {
                cells.add(GridCell(native.unvisitedRows[i], native.unvisitedCols[i]))
            }
            return CoverageResult(fraction = native.fraction, unvisited = cells)
        }
        val fallback = kotlinCoverageFallback(geofence, route)
        val total = COVERAGE_ROWS * COVERAGE_COLS
        val visited = total - fallback.size
        val fraction = if (total == 0) 0.0 else visited.toDouble() / total
        return CoverageResult(fraction = fraction, unvisited = fallback)
    }

    fun saveGeofence(bounds: GeofenceBounds) {
        if (sessionIdState.value != null) {
            WardriveSessionService.configureGeofence(
                appContext,
                bounds.latMin,
                bounds.latMax,
                bounds.lonMin,
                bounds.lonMax,
            )
        } else {
            viewModelScope.launch {
                appPreferences.saveGeofence(bounds)
                gatewayWifi.applyStoredGeofencePolicy()
                _state.update { it.copy(geofence = bounds) }
                scheduleCoverage()
            }
        }
    }

    fun clearGeofence() {
        if (sessionIdState.value != null) {
            WardriveSessionService.clearGeofence(appContext)
        } else {
            viewModelScope.launch {
                appPreferences.clearGeofence()
                gatewayWifi.applyStoredGeofencePolicy()
                _state.update {
                    it.copy(geofence = null, coverageUnvisited = emptyList(), coverageFraction = 0.0)
                }
            }
        }
    }

    private fun kotlinCoverageFallback(
        geofence: GeofenceBounds,
        route: List<RoutePoint>,
    ): List<GridCell> {
        val visited = HashSet<Pair<Int, Int>>()
        val latSpan = geofence.latMax - geofence.latMin
        val lonSpan = geofence.lonMax - geofence.lonMin
        if (latSpan <= 0.0 || lonSpan <= 0.0) return emptyList()
        for (pt in route) {
            if (pt.lat < geofence.latMin || pt.lat > geofence.latMax) continue
            if (pt.lon < geofence.lonMin || pt.lon > geofence.lonMax) continue
            val row =
                ((pt.lat - geofence.latMin) / latSpan * COVERAGE_ROWS)
                    .toInt()
                    .coerceIn(0, COVERAGE_ROWS - 1)
            val col =
                ((pt.lon - geofence.lonMin) / lonSpan * COVERAGE_COLS)
                    .toInt()
                    .coerceIn(0, COVERAGE_COLS - 1)
            visited.add(row to col)
        }
        val out = mutableListOf<GridCell>()
        for (row in 0 until COVERAGE_ROWS) {
            for (col in 0 until COVERAGE_COLS) {
                if ((row to col) !in visited) out.add(GridCell(row, col))
            }
        }
        return out
    }

    class Factory(
        private val db: WardriverDatabase,
        private val appContext: Context,
    ) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            return MapViewModel(db, appContext) as T
        }
    }

    companion object {
        const val COVERAGE_ROWS = 12
        const val COVERAGE_COLS = 12
        private const val GPS_CENTER_MAX_ACCURACY_M = 100f
        private const val VIEWPORT_DEBOUNCE_MS = 300L
        private const val VIEWPORT_SAMPLE_MS_ALL = 500L
        private const val MAP_UPDATE_DEBOUNCE_MS = 500L
        private const val MAP_UPDATE_DEBOUNCE_MS_ALL = 750L
        private const val CLUSTER_REBUILD_DEBOUNCE_MS = 200L
        private const val CLUSTER_REBUILD_DEBOUNCE_MS_ALL = 400L
        private const val COVERAGE_DEBOUNCE_MS = 1_000L

        internal fun resolveDisplaySessionId(selectedId: Long?, latestId: Long?): Long? =
            selectedId ?: latestId
    }
}

/** Mean lat/lon of AP markers with non-zero coordinates. */
private fun centroidFromApMarkers(markers: List<ApMarker>): GeoPoint? {
    val valid = markers.filter { it.lat != 0.0 || it.lon != 0.0 }
    if (valid.isEmpty()) return null
    val lat = valid.map { it.lat }.average()
    val lon = valid.map { it.lon }.average()
    return GeoPoint(lat, lon)
}

private fun centroidFromRoute(points: List<GeoPoint>): GeoPoint? {
    if (points.isEmpty()) return null
    if (points.size == 1) return points.last()
    val lat = points.map { it.latitude }.average()
    val lon = points.map { it.longitude }.average()
    return GeoPoint(lat, lon)
}

private fun geofenceCenter(bounds: GeofenceBounds): GeoPoint =
    GeoPoint(
        (bounds.latMin + bounds.latMax) / 2.0,
        (bounds.lonMin + bounds.lonMax) / 2.0,
    )

private fun resolveCenterTarget(
    userLocation: GeoPoint?,
    gpsUsable: Boolean,
    routePoints: List<GeoPoint>,
    apCentroid: GeoPoint?,
    homeZoneCenter: GeoPoint?,
    geofence: GeofenceBounds?,
): MapCenterTarget? {
    if (gpsUsable && userLocation != null) {
        return MapCenterTarget(userLocation, zoom = 15.0, source = CenterSource.Gps)
    }
    centroidFromRoute(routePoints)?.let { point ->
        return MapCenterTarget(point, zoom = 15.0, source = CenterSource.Route)
    }
    apCentroid?.let { point ->
        return MapCenterTarget(point, zoom = 14.0, source = CenterSource.AccessPoints)
    }
    homeZoneCenter?.let { point ->
        return MapCenterTarget(point, zoom = 14.0, source = CenterSource.HomeZone)
    }
    geofence?.let { bounds ->
        return MapCenterTarget(geofenceCenter(bounds), zoom = 13.0, source = CenterSource.Geofence)
    }
    return null
}

data class MapCenterTarget(
    val point: GeoPoint,
    val zoom: Double = 15.0,
    val source: CenterSource,
)

enum class CenterSource {
    Gps,
    Route,
    AccessPoints,
    HomeZone,
    Geofence,
}

data class MapUiState(
    val displaySessionId: Long? = null,
    val selectedSessionId: Long? = null,
    val hasActiveWardrive: Boolean = false,
    val dataScope: MapDataScope = MapDataScope.Session,
    val layerFilters: MapLayerFilters = MapLayerFilters(),
    val layerPanelExpanded: Boolean = false,
    val coverageShadingEnabled: Boolean = true,
    val mapBasemap: MapBasemap = MapBasemap.Light,
    val geofenceEditorVisible: Boolean = true,
    val searchQuery: String = "",
    val searchResultCount: Int = 0,
    val routePoints: List<GeoPoint> = emptyList(),
    val rawRouteCount: Int = 0,
    val accessPointClusters: List<ApCluster> = emptyList(),
    val selectedCluster: ApCluster? = null,
    val selectedAlertPin: AlertPin? = null,
    val remoteIdOperatorPoint: GeoPoint? = null,
    val remoteIdFocusNonce: Int = 0,
    val geofence: GeofenceBounds? = null,
    val coverageUnvisited: List<GridCell> = emptyList(),
    val coverageFraction: Double = 0.0,
    val alertPins: List<AlertPin> = emptyList(),
    val zoomLevel: Double = 15.0,
    val mapCenterTarget: MapCenterTarget? = null,
    val userLocation: GeoPoint? = null,
    val recenterNonce: Int = 0,
    val driverLockEnabled: Boolean = false,
    val isLoading: Boolean = false,
    val loadingCount: Int? = null,
)

data class GeofenceBounds(
    val latMin: Double,
    val latMax: Double,
    val lonMin: Double,
    val lonMax: Double,
)

data class GridCell(val row: Int, val col: Int)

data class AlertPin(
    val id: String,
    val lat: Double,
    val lon: Double,
    val kind: String,
    val title: String,
    val detail: String,
    val unixMs: Long,
    val detectionMethodId: Int? = null,
    val remoteId: RemoteIdDetail? = null,
)
