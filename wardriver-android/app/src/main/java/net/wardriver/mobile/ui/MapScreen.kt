package net.wardriver.mobile.ui

import android.view.MotionEvent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.EditLocation
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.GpsFixed
import androidx.compose.material.icons.filled.MyLocation
import androidx.compose.material.icons.outlined.Label
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.FloatingActionButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.keepScreenOn
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.viewmodel.compose.viewModel
import net.wardriver.mobile.data.AlertKind
import net.wardriver.mobile.db.WardriverDatabase
import net.wardriver.mobile.ui.map.GeofenceEditor
import net.wardriver.mobile.ui.map.MapAttribution
import net.wardriver.mobile.ui.map.MapBasemap
import net.wardriver.mobile.ui.map.MapAlertDetailSheet
import net.wardriver.mobile.ui.map.MapDeviceDetailSheet
import net.wardriver.mobile.ui.map.MapOverlayController
import net.wardriver.mobile.ui.map.MapTileCache
import net.wardriver.mobile.ui.map.MapUiState
import net.wardriver.mobile.ui.map.MapViewModel
import net.wardriver.mobile.ui.map.MapViewport
import net.wardriver.mobile.ui.map.MapLayerSheet
import net.wardriver.mobile.ui.map.SessionPickerSheet
import net.wardriver.mobile.ui.map.PackTileSources
import net.wardriver.mobile.ui.map.REMOTE_ID_PANEL_HEIGHT_FRACTION
import net.wardriver.mobile.ui.map.RemoteIdMapDetailPanel
import net.wardriver.mobile.ui.map.rememberGeofenceDrawState
import net.wardriver.mobile.ui.map.remoteIdPanelBottomPaddingPx
import net.wardriver.mobile.ui.theme.PackColor
import org.osmdroid.config.Configuration
import org.osmdroid.events.MapListener
import org.osmdroid.util.BoundingBox
import org.osmdroid.util.GeoPoint
import org.osmdroid.views.MapView
import org.osmdroid.views.overlay.mylocation.GpsMyLocationProvider
import org.osmdroid.views.overlay.mylocation.MyLocationNewOverlay
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val SESSION_DATE_FORMAT = SimpleDateFormat("MMM d, yyyy h:mm a", Locale.getDefault())

@Composable
fun MapScreen(
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val db = remember { WardriverDatabase.getInstance(context) }
    val viewModel: MapViewModel =
        viewModel(factory = MapViewModel.Factory(db, context.applicationContext))
    val state by viewModel.state.collectAsState()
    val sessions by viewModel.sessions.collectAsState(emptyList())
    var geofenceEditMode by remember { mutableStateOf(false) }
    val drawState = rememberGeofenceDrawState()

    // Sheet visibility state
    var showLayerSheet by remember { mutableStateOf(state.layerPanelExpanded) }
    var showSessionPicker by remember { mutableStateOf(false) }

    LaunchedEffect(state.layerPanelExpanded) {
        showLayerSheet = state.layerPanelExpanded
    }

    // Load sessions when picker is opened
    LaunchedEffect(showSessionPicker) {
        if (showSessionPicker) {
            viewModel.loadSessions()
        }
    }

    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                when (event) {
                    Lifecycle.Event.ON_RESUME -> {
                        viewModel.setMapActive(true)
                        viewModel.onMapVisible()
                    }
                    Lifecycle.Event.ON_STOP -> viewModel.setMapActive(false)
                    else -> Unit
                }
            }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            viewModel.setMapActive(false)
        }
    }

    LaunchedEffect(Unit) {
        viewModel.setMapActive(true)
    }

    LaunchedEffect(geofenceEditMode) {
        if (geofenceEditMode && state.driverLockEnabled) {
            viewModel.setDriverLock(false)
        }
    }

    val keepAwake = state.hasActiveWardrive && state.driverLockEnabled
    Box(
        modifier
            .fillMaxSize()
            .then(if (keepAwake) Modifier.keepScreenOn() else Modifier),
    ) {
        // OSMDroid map (zIndex 0)
        OsmdroidMap(
            state = state,
            geofenceEditMode = geofenceEditMode,
            drawState = drawState,
            onZoomChanged = viewModel::setZoomLevel,
            onViewportChanged = viewModel::onViewportChanged,
            onClusterClick = viewModel::selectCluster,
            onAlertClick = viewModel::selectAlertPin,
            onGeofenceDrawn = { bounds ->
                viewModel.saveGeofence(bounds)
                geofenceEditMode = false
            },
            modifier =
                Modifier
                    .fillMaxSize()
                    .zIndex(0f)
                    .clipToBounds(),
        )

        // Loading overlay (zIndex 3) — above map, below sheets
        if (state.isLoading) {
            Box(
                Modifier
                    .fillMaxSize()
                    .zIndex(3f)
                    .background(Color(0x99000000f)),
                contentAlignment = Alignment.Center,
            ) {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    CircularProgressIndicator(color = PackColor.Cyan)
                    Text(
                        text = if (state.loadingCount != null && state.loadingCount!! > 0) {
                            "Loading ${state.loadingCount!!} devices, ${state.alertPins.size} alerts..."
                        } else {
                            "Loading..."
                        },
                        color = Color.White,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
        }

        // Top-left column: banner + geofence editor (zIndex 1)
        Column(
            Modifier
                .zIndex(1f)
                .align(Alignment.TopStart),
        ) {
            // Session banner
            if (!state.hasActiveWardrive) {
                val bannerText = if (state.dataScope == net.wardriver.mobile.ui.map.MapDataScope.AllDevices) {
                    "Showing all devices"
                } else if (state.selectedSessionId != null) {
                    val selected = sessions.find { it.session.id == state.selectedSessionId }
                    selected?.session?.let {
                        "Showing session from ${SESSION_DATE_FORMAT.format(Date(it.startedUnixMs))}"
                    } ?: "No active wardrive - showing last session"
                } else if (state.displaySessionId != null) {
                    "No active wardrive - showing last session"
                } else {
                    "No active wardrive"
                }
                Text(
                    text = bannerText,
                    style = MaterialTheme.typography.labelMedium,
                    color = PackColor.Sun,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .background(PackColor.Surface2)
                            .padding(horizontal = 12.dp, vertical = 6.dp),
                )
            }

            // Geofence editor (gated behind toggle)
            if (state.geofenceEditorVisible && state.geofence != null) {
                GeofenceEditor(
                    geofence = state.geofence,
                    coverageFraction = state.coverageFraction,
                    editMode = geofenceEditMode,
                    onEditModeChange = { enabled ->
                        geofenceEditMode = enabled
                        if (enabled) {
                            viewModel.setDriverLock(false)
                        }
                        if (!enabled) drawState.reset()
                    },
                    onClear = { viewModel.clearGeofence() },
                )
            }
        }

        // Bottom-right FAB column (zIndex 2) — order: bottom to top
        val selectedRemoteId = state.selectedAlertPin?.takeIf { it.kind == AlertKind.REMOTE_ID }
        val ridPanelBottomPad =
            if (selectedRemoteId != null) {
                (LocalConfiguration.current.screenHeightDp * REMOTE_ID_PANEL_HEIGHT_FRACTION).dp
            } else {
                0.dp
            }
        Column(
            Modifier
                .zIndex(2f)
                .align(Alignment.BottomEnd)
                .padding(16.dp)
                .padding(bottom = ridPanelBottomPad),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // Driver lock (only when active wardrive)
            if (state.hasActiveWardrive) {
                val lockOn = state.driverLockEnabled
                FloatingActionButton(
                    onClick = { viewModel.toggleDriverLock() },
                    containerColor = if (lockOn) PackColor.Cyan else PackColor.Surface2,
                    contentColor = if (lockOn) PackColor.Background else PackColor.Muted,
                    elevation = FloatingActionButtonDefaults.elevation(),
                    modifier =
                        if (lockOn) {
                            Modifier
                        } else {
                            Modifier.border(
                                width = 1.dp,
                                color = PackColor.BorderCyan,
                                shape = FloatingActionButtonDefaults.shape,
                            )
                        },
                ) {
                    Icon(
                        Icons.Filled.GpsFixed,
                        contentDescription =
                            if (lockOn) "Unlock map from driver" else "Lock map on driver",
                        tint = if (lockOn) PackColor.Background else PackColor.Muted,
                    )
                }
            }

            // Geofence editor toggle (fixed slot — prevents column jump)
            if (state.geofence != null) {
                FloatingActionButton(
                    onClick = { viewModel.toggleGeofenceEditorVisibility() },
                    containerColor = if (state.geofenceEditorVisible) PackColor.Cyan else PackColor.Surface2,
                    contentColor = if (state.geofenceEditorVisible) PackColor.Background else PackColor.Muted,
                    elevation = FloatingActionButtonDefaults.elevation(),
                ) {
                    Icon(
                        Icons.Filled.EditLocation,
                        contentDescription = if (state.geofenceEditorVisible) "Hide editor" else "Show editor",
                        tint = if (state.geofenceEditorVisible) PackColor.Background else PackColor.Muted,
                    )
                }
            } else {
                Spacer(Modifier.size(40.dp)) // Fixed-height slot prevents column shift
            }

            // Session picker FAB
            if (!state.hasActiveWardrive) {
                FloatingActionButton(
                    onClick = {
                        showLayerSheet = false
                        viewModel.setLayerPanelExpanded(false)
                        showSessionPicker = true
                    },
                    containerColor = PackColor.Surface2,
                    contentColor = PackColor.Cyan,
                    elevation = FloatingActionButtonDefaults.elevation(),
                ) {
                    Icon(
                        Icons.Outlined.Label,
                        contentDescription = "Select session",
                        tint = PackColor.Cyan,
                    )
                    Text("S", color = PackColor.Cyan, modifier = Modifier.padding(start = 2.dp))
                }
            } else {
                Spacer(Modifier.size(40.dp))
            }

            // Layer filters FAB
            FloatingActionButton(
                onClick = {
                    showSessionPicker = false
                    showLayerSheet = !showLayerSheet
                    viewModel.setLayerPanelExpanded(showLayerSheet)
                },
                containerColor = if (showLayerSheet) PackColor.Cyan else PackColor.Surface2,
                contentColor = if (showLayerSheet) PackColor.Background else PackColor.Cyan,
                elevation = FloatingActionButtonDefaults.elevation(),
            ) {
                Icon(
                    Icons.Filled.FilterList,
                    contentDescription = "Map layers",
                    tint = if (showLayerSheet) PackColor.Background else PackColor.Cyan,
                )
            }

            // Recenter (always visible, at bottom)
            FloatingActionButton(
                onClick = { viewModel.recenter() },
                containerColor = PackColor.Cyan,
                contentColor = PackColor.Background,
                elevation = FloatingActionButtonDefaults.elevation(),
            ) {
                Icon(
                    Icons.Filled.MyLocation,
                    contentDescription = "Recenter map",
                    tint = PackColor.Background,
                )
            }
        }

        // Attribution (zIndex 2)
        MapAttribution(
            basemap = state.mapBasemap,
            modifier =
                Modifier
                    .zIndex(2f)
                    .align(Alignment.BottomStart)
                    .padding(start = 8.dp, bottom = 8.dp + ridPanelBottomPad),
        )

        // Device detail sheet
        state.selectedCluster?.let { cluster ->
            MapDeviceDetailSheet(
                cluster = cluster,
                onDismiss = viewModel::clearSelectedCluster,
            )
        }

        // Alert detail sheet (non-Remote-ID)
        val selectedOtherAlert = state.selectedAlertPin?.takeIf { it.kind != AlertKind.REMOTE_ID }
        selectedOtherAlert?.let { pin ->
            MapAlertDetailSheet(
                pin = pin,
                onDismiss = viewModel::clearSelectedAlertPin,
            )
        }

        // Remote ID panel
        selectedRemoteId?.let { pin ->
            RemoteIdMapDetailPanel(
                pin = pin,
                onDismiss = viewModel::clearSelectedAlertPin,
                modifier =
                    Modifier
                        .zIndex(3f)
                        .align(Alignment.BottomCenter)
                        .fillMaxWidth(),
            )
        }

        // Layer filters sheet (mutually exclusive with session picker)
        if (showLayerSheet) {
            MapLayerSheet(
                layerFilters = state.layerFilters,
                searchQuery = state.searchQuery,
                searchResultCount = state.searchResultCount,
                geofenceActive = state.geofence != null,
                coverageShadingEnabled = state.coverageShadingEnabled,
                onLayerFiltersChange = viewModel::setLayerFilters,
                onSearchQueryChange = viewModel::setSearchQuery,
                onCoverageShadingChange = viewModel::setCoverageShadingEnabled,
                onDismiss = {
                    showLayerSheet = false
                    viewModel.setLayerPanelExpanded(false)
                },
            )
        }

        // Session picker sheet (mutually exclusive with layer sheet)
        if (showSessionPicker) {
            SessionPickerSheet(
                sessions = sessions,
                hasActiveWardrive = state.hasActiveWardrive,
                selectedSessionId = state.selectedSessionId,
                dataScope = state.dataScope,
                onSelectSession = { id ->
                    viewModel.setSelectedSessionId(id)
                    // Picking a session implies Session scope
                    viewModel.setDataScope(net.wardriver.mobile.ui.map.MapDataScope.Session)
                    showSessionPicker = false
                },
                onSelectAllDevices = {
                    viewModel.setSelectedSessionId(null)
                    viewModel.setDataScope(net.wardriver.mobile.ui.map.MapDataScope.AllDevices)
                    showSessionPicker = false
                },
                onDismiss = { showSessionPicker = false },
            )
        }
    }
}

@Composable
private fun OsmdroidMap(
    state: MapUiState,
    geofenceEditMode: Boolean,
    drawState: net.wardriver.mobile.ui.map.GeofenceDrawState,
    onZoomChanged: (Double) -> Unit,
    onViewportChanged: (MapViewport) -> Unit,
    onClusterClick: (net.wardriver.mobile.ui.map.ApCluster) -> Unit,
    onAlertClick: (net.wardriver.mobile.ui.map.AlertPin) -> Unit,
    onGeofenceDrawn: (net.wardriver.mobile.ui.map.GeofenceBounds) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val onZoom by rememberUpdatedState(onZoomChanged)
    val onViewport by rememberUpdatedState(onViewportChanged)
    val onClusterTap by rememberUpdatedState(onClusterClick)
    val onAlertTap by rememberUpdatedState(onAlertClick)
    var hasInitialCentered by remember { mutableStateOf(false) }
    var lastAppliedRecenterNonce by remember { mutableIntStateOf(-1) }
    val overlayHolder = remember { arrayOf<MyLocationNewOverlay?>(null) }
    val mapView =
        remember {
            Configuration.getInstance().userAgentValue = context.packageName
            MapView(context).apply {
                setTileSource(PackTileSources.tileSource(MapBasemap.Light))
                controller.setZoom(15.0)
                setMultiTouchControls(true)
                val overlay =
                    MyLocationNewOverlay(GpsMyLocationProvider(context), this).apply {
                        enableMyLocation()
                    }
                overlayHolder[0] = overlay
                overlays.add(overlay)
                addMapListener(
                    object : MapListener {
                        override fun onScroll(event: org.osmdroid.events.ScrollEvent?): Boolean {
                            publishMapViewState(this@apply, onZoom, onViewport)
                            return false
                        }

                        override fun onZoom(event: org.osmdroid.events.ZoomEvent?): Boolean {
                            publishMapViewState(this@apply, onZoom, onViewport)
                            return false
                        }
                    },
                )
            }
        }
    val overlayController = remember(mapView) { MapOverlayController(mapView) }
    val geofenceCornerA = drawState.cornerA
    val geofenceCornerB = drawState.cornerB
    var appliedBasemap by remember { mutableStateOf<MapBasemap?>(null) }

    LaunchedEffect(Unit) {
        MapTileCache.purgeLegacyMapnikOnce(context)
    }

    LaunchedEffect(state.mapBasemap) {
        val basemap = state.mapBasemap
        if (appliedBasemap == basemap) return@LaunchedEffect
        mapView.setTileSource(PackTileSources.tileSource(basemap))
        mapView.invalidate()
        appliedBasemap = basemap
    }

    LaunchedEffect(state.mapCenterTarget, state.recenterNonce) {
        val target = state.mapCenterTarget ?: return@LaunchedEffect
        val shouldApply = !hasInitialCentered || state.recenterNonce != lastAppliedRecenterNonce
        if (shouldApply) {
            mapView.controller.setZoom(target.zoom)
            mapView.controller.setCenter(target.point)
            hasInitialCentered = true
            lastAppliedRecenterNonce = state.recenterNonce
            publishMapViewState(mapView, onZoom, onViewport)
        }
    }

    LaunchedEffect(state.driverLockEnabled, state.userLocation) {
        val overlay = overlayHolder[0] ?: return@LaunchedEffect
        if (state.driverLockEnabled && state.userLocation != null) {
            overlay.enableFollowLocation()
            mapView.controller.setCenter(state.userLocation)
        } else {
            overlay.disableFollowLocation()
        }
    }

    LaunchedEffect(state.routePoints) {
        overlayController.syncRoute(state.routePoints)
    }

    LaunchedEffect(state.geofence) {
        overlayController.syncGeofence(state.geofence)
    }

    LaunchedEffect(state.geofence, state.coverageUnvisited) {
        overlayController.syncCoverage(state.geofence, state.coverageUnvisited)
    }

    LaunchedEffect(state.layerFilters, state.dataScope, state.accessPointClusters) {
        overlayController.syncClusters(state.accessPointClusters) { cluster ->
            onClusterTap(cluster)
        }
    }

    LaunchedEffect(state.layerFilters, state.dataScope, state.alertPins) {
        overlayController.syncAlerts(state.alertPins) { pin ->
            onAlertTap(pin)
        }
    }

    LaunchedEffect(state.remoteIdOperatorPoint) {
        overlayController.syncOperatorPin(state.remoteIdOperatorPoint)
    }

    LaunchedEffect(state.remoteIdFocusNonce, state.selectedAlertPin, state.remoteIdOperatorPoint) {
        val pin = state.selectedAlertPin
        if (pin == null || pin.kind != AlertKind.REMOTE_ID) {
            mapView.setMapCenterOffset(0, 0)
            return@LaunchedEffect
        }
        val bottomPad = remoteIdPanelBottomPaddingPx(mapView.height.coerceAtLeast(1))
        // Shift visual center upward so fit targets the map area above the panel.
        mapView.setMapCenterOffset(0, -(bottomPad / 2))
        val drone = GeoPoint(pin.lat, pin.lon)
        val operator = state.remoteIdOperatorPoint
        val bbox = mapView.boundingBox
        val droneVisible = bbox.contains(drone)
        val operatorVisible = operator?.let { bbox.contains(it) } ?: true
        if (!droneVisible || !operatorVisible) {
            val points = buildList {
                add(drone)
                operator?.let { add(it) }
            }
            val north = points.maxOf { it.latitude }
            val south = points.minOf { it.latitude }
            val east = points.maxOf { it.longitude }
            val west = points.minOf { it.longitude }
            val bounds = BoundingBox(north, east, south, west)
            mapView.zoomToBoundingBox(bounds, true, 96)
            publishMapViewState(mapView, onZoom, onViewport)
        }
    }

    LaunchedEffect(geofenceEditMode, geofenceCornerA, geofenceCornerB) {
        val preview = if (geofenceEditMode) drawState.previewBounds() else null
        overlayController.syncGeofencePreview(preview)
    }

    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                when (event) {
                    Lifecycle.Event.ON_RESUME -> {
                        mapView.onResume()
                        publishMapViewState(mapView, onZoom, onViewport)
                    }
                    Lifecycle.Event.ON_PAUSE -> mapView.onPause()
                    else -> Unit
                }
            }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            mapView.onPause()
        }
    }

    AndroidView(
        modifier = modifier,
        factory = { mapView },
        update = { map ->
            if (geofenceEditMode) {
                map.setOnTouchListener { _, event ->
                    if (event.action == MotionEvent.ACTION_UP) {
                        val proj = map.projection
                        val geo = proj.fromPixels(event.x.toInt(), event.y.toInt()) as GeoPoint
                        drawState.onMapTap(geo) { bounds ->
                            onGeofenceDrawn(bounds)
                        }
                        map.invalidate()
                    }
                    false
                }
            } else {
                map.setOnTouchListener(null)
            }
        },
    )
}

private fun publishMapViewState(
    map: MapView,
    onZoom: (Double) -> Unit,
    onViewport: (MapViewport) -> Unit,
) {
    onZoom(map.zoomLevelDouble)
    val bbox = map.boundingBox
    onViewport(
        MapViewport(
            south = bbox.latSouth,
            north = bbox.latNorth,
            west = bbox.lonWest,
            east = bbox.lonEast,
        ),
    )
}
