package net.wardriver.mobile.data

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.MutablePreferences
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import net.wardriver.mobile.ui.map.GeofenceBounds
import net.wardriver.mobile.ui.map.MapBasemap
import net.wardriver.mobile.ui.map.MapDataScope
import net.wardriver.mobile.ui.map.MapLayerFilters

private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "wardriver_prefs")

/**
 * DataStore-backed app preferences for settings hub, geofence, uploads, and policy_sync inputs.
 * Applied to the gateway on the next PackLink policy_sync push when connected.
 */
class AppPreferences(private val context: Context) {
    private val dataStore = context.dataStore

    // --- Geofence (map bounds for gateway policy_sync) ---

    /** Live geofence bounds from prefs (null when disabled or unset). */
    val geofenceFlow: Flow<GeofenceBounds?> =
        dataStore.data.map { prefs ->
            if (prefs[KEY_GEOFENCE_ENABLED] != true) return@map null
            val latMin = prefs[KEY_GEOFENCE_LAT_MIN] ?: return@map null
            val latMax = prefs[KEY_GEOFENCE_LAT_MAX] ?: return@map null
            val lonMin = prefs[KEY_GEOFENCE_LON_MIN] ?: return@map null
            val lonMax = prefs[KEY_GEOFENCE_LON_MAX] ?: return@map null
            if (latMin >= latMax || lonMin >= lonMax) return@map null
            GeofenceBounds(latMin, latMax, lonMin, lonMax)
        }

    suspend fun saveGeofence(bounds: GeofenceBounds) {
        dataStore.edit { prefs ->
            prefs[KEY_GEOFENCE_ENABLED] = true
            prefs[KEY_GEOFENCE_LAT_MIN] = bounds.latMin
            prefs[KEY_GEOFENCE_LAT_MAX] = bounds.latMax
            prefs[KEY_GEOFENCE_LON_MIN] = bounds.lonMin
            prefs[KEY_GEOFENCE_LON_MAX] = bounds.lonMax
        }
    }

    suspend fun clearGeofence() {
        dataStore.edit { prefs ->
            prefs[KEY_GEOFENCE_ENABLED] = false
            prefs.remove(KEY_GEOFENCE_LAT_MIN)
            prefs.remove(KEY_GEOFENCE_LAT_MAX)
            prefs.remove(KEY_GEOFENCE_LON_MIN)
            prefs.remove(KEY_GEOFENCE_LON_MAX)
        }
    }

    // --- Map layer filters and data scope ---

    val mapDataScopeFlow: Flow<MapDataScope> =
        dataStore.data.map { prefs ->
            when (prefs[KEY_MAP_DATA_SCOPE]) {
                "all_devices" -> MapDataScope.AllDevices
                else -> MapDataScope.Session
            }
        }

    val mapLayerFiltersFlow: Flow<MapLayerFilters> =
        dataStore.data.map { prefs ->
            // Read detection filter keys (new key wins; legacy flock_ key readable).
            val oldFlockWifi =
                prefs[KEY_MAP_FILTER_DETECTION_WIFI] ?: prefs[KEY_LEGACY_MAP_FILTER_FLOCK_WIFI]
            val oldFlockBle =
                prefs[KEY_MAP_FILTER_DETECTION_BLE] ?: prefs[KEY_LEGACY_MAP_FILTER_FLOCK_BLE]
            val oldAlerts = prefs[KEY_MAP_FILTER_ALERTS]
            val oldSsidWatch = prefs[KEY_MAP_FILTER_SSID_WATCH]
            val oldRemoteId = prefs[KEY_MAP_FILTER_REMOTE_ID]

            // Determine if old-style prefs exist.
            val hasOldPrefs = (oldFlockWifi != null || oldFlockBle != null)

            // Master alerts: new key wins; else migrate from old; else default true.
            val masterAlerts = prefs[KEY_MAP_FILTER_ALERTS] != false

            // Per-type: new key wins; else derive from master/old keys; else default true.
            val alertDetectionWifi = when {
                prefs[KEY_MAP_FILTER_ALERT_DETECTION_WIFI] != null -> prefs[KEY_MAP_FILTER_ALERT_DETECTION_WIFI] == true
                prefs[KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_WIFI] != null -> prefs[KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_WIFI] == true
                hasOldPrefs -> (oldFlockWifi == true) or (oldFlockBle == true)
                else -> masterAlerts
            }
            val alertDetectionBle = when {
                prefs[KEY_MAP_FILTER_ALERT_DETECTION_BLE] != null -> prefs[KEY_MAP_FILTER_ALERT_DETECTION_BLE] == true
                prefs[KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_BLE] != null -> prefs[KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_BLE] == true
                hasOldPrefs -> (oldFlockWifi == true) or (oldFlockBle == true)
                else -> masterAlerts
            }
            val alertSsidWatch = when {
                prefs[KEY_MAP_FILTER_ALERT_SSID_WATCH] != null -> prefs[KEY_MAP_FILTER_ALERT_SSID_WATCH] == true
                oldSsidWatch != null -> oldSsidWatch
                else -> masterAlerts
            }
            val alertRemoteId = when {
                prefs[KEY_MAP_FILTER_ALERT_REMOTE_ID] != null -> prefs[KEY_MAP_FILTER_ALERT_REMOTE_ID] == true
                oldRemoteId != null -> oldRemoteId
                else -> masterAlerts
            }
            val alertAxon = prefs[KEY_MAP_FILTER_ALERT_AXON] != false
            val alertIPro = prefs[KEY_MAP_FILTER_ALERT_IPRO] != false

            MapLayerFilters(
                wifi = prefs[KEY_MAP_FILTER_WIFI] != false,
                ble = prefs[KEY_MAP_FILTER_BLE] != false,
                probe = prefs[KEY_MAP_FILTER_PROBE] != false,
                alerts = masterAlerts,
                alertDetectionWifi = alertDetectionWifi,
                alertDetectionBle = alertDetectionBle,
                alertSsidWatch = alertSsidWatch,
                alertRemoteId = alertRemoteId,
                alertAxon = alertAxon,
                alertIPro = alertIPro,
            )
        }

    val mapCoverageShadingEnabledFlow: Flow<Boolean> =
        dataStore.data.map { prefs -> prefs[KEY_MAP_COVERAGE_SHADING] != false }

    // --- Map layer panel expanded state (collapsed behind FAB) ---

    val mapLayerPanelExpandedFlow: Flow<Boolean> =
        dataStore.data.map { prefs -> prefs[KEY_MAP_LAYER_PANEL_EXPANDED] == true }

    suspend fun setMapLayerPanelExpanded(expanded: Boolean) {
        dataStore.edit { prefs -> prefs[KEY_MAP_LAYER_PANEL_EXPANDED] = expanded }
    }

    suspend fun setMapLayerFilters(filters: MapLayerFilters) {
        dataStore.edit { prefs ->
            prefs[KEY_MAP_FILTER_WIFI] = filters.wifi
            prefs[KEY_MAP_FILTER_BLE] = filters.ble
            prefs[KEY_MAP_FILTER_PROBE] = filters.probe
            prefs[KEY_MAP_FILTER_ALERTS] = filters.alerts
            prefs[KEY_MAP_FILTER_ALERT_DETECTION_WIFI] = filters.alertDetectionWifi
            prefs[KEY_MAP_FILTER_ALERT_DETECTION_BLE] = filters.alertDetectionBle
            prefs[KEY_MAP_FILTER_ALERT_SSID_WATCH] = filters.alertSsidWatch
            prefs[KEY_MAP_FILTER_ALERT_REMOTE_ID] = filters.alertRemoteId
            prefs[KEY_MAP_FILTER_ALERT_AXON] = filters.alertAxon
            prefs[KEY_MAP_FILTER_ALERT_IPRO] = filters.alertIPro
        }
    }

    suspend fun setMapCoverageShadingEnabled(enabled: Boolean) {
        dataStore.edit { prefs -> prefs[KEY_MAP_COVERAGE_SHADING] = enabled }
    }

    val mapBasemapFlow: Flow<MapBasemap> =
        dataStore.data.map { prefs ->
            when (prefs[KEY_MAP_BASEMAP]) {
                "dark" -> MapBasemap.Dark
                else -> MapBasemap.Light
            }
        }

    suspend fun setMapBasemap(basemap: MapBasemap) {
        dataStore.edit { prefs ->
            prefs[KEY_MAP_BASEMAP] =
                when (basemap) {
                    MapBasemap.Dark -> "dark"
                    MapBasemap.Light -> "light"
                }
        }
    }

    // --- Gateway SoftAP credentials (join + optional push to gateway NVS) ---

    data class GatewaySoftApCreds(
        val ssid: String,
        val password: String,
    )

    val gatewaySoftApSsidFlow: Flow<String> =
        dataStore.data.map { it[KEY_GATEWAY_SOFTAP_SSID] ?: DEFAULT_GATEWAY_SOFTAP_SSID }

    val gatewaySoftApPasswordFlow: Flow<String> =
        dataStore.data.map { it[KEY_GATEWAY_SOFTAP_PASS] ?: DEFAULT_GATEWAY_SOFTAP_PASS }

    suspend fun gatewaySoftApCreds(): GatewaySoftApCreds {
        val prefs = dataStore.data.first()
        return GatewaySoftApCreds(
            ssid = prefs[KEY_GATEWAY_SOFTAP_SSID] ?: DEFAULT_GATEWAY_SOFTAP_SSID,
            password = prefs[KEY_GATEWAY_SOFTAP_PASS] ?: DEFAULT_GATEWAY_SOFTAP_PASS,
        )
    }

    suspend fun saveGatewaySoftApCreds(ssid: String, password: String) {
        dataStore.edit { prefs ->
            prefs[KEY_GATEWAY_SOFTAP_SSID] = ssid
            prefs[KEY_GATEWAY_SOFTAP_PASS] = password
        }
    }

    // --- Preferred gateway (legacy key retained for migration) ---

    // --- Fleet scan mode (dashboard Pause / Wardrive toggles) ---

    val scanAllowedFlow: Flow<Boolean> =
        dataStore.data.map { prefs -> prefs[KEY_SCAN_ALLOWED] != false }

    suspend fun setScanAllowed(allowed: Boolean) {
        dataStore.edit { prefs -> prefs[KEY_SCAN_ALLOWED] = allowed }
    }

    suspend fun isScanAllowed(): Boolean = scanAllowedFlow.first()

    // --- Wardrive session persistence (process death resume) ---

    suspend fun setWardriveActive(
        active: Boolean,
        sessionId: Long? = null,
        gatewayMac: String? = null,
    ) {
        dataStore.edit { prefs ->
            prefs[KEY_WARDRIVE_ACTIVE] = active
            if (active && sessionId != null) {
                prefs[KEY_ACTIVE_WARDRIVE_SESSION_ID] = sessionId
            } else if (!active) {
                prefs.remove(KEY_ACTIVE_WARDRIVE_SESSION_ID)
                prefs.remove(KEY_ACTIVE_SESSION_GATEWAY_MAC)
            }
            if (active && !gatewayMac.isNullOrBlank()) {
                prefs[KEY_ACTIVE_SESSION_GATEWAY_MAC] = gatewayMac
            }
        }
    }

    suspend fun wardriveActive(): Boolean = dataStore.data.first()[KEY_WARDRIVE_ACTIVE] == true

    suspend fun activeWardriveSessionId(): Long? =
        dataStore.data.first()[KEY_ACTIVE_WARDRIVE_SESSION_ID]

    suspend fun activeSessionGatewayMac(): String? =
        dataStore.data.first()[KEY_ACTIVE_SESSION_GATEWAY_MAC]

    // --- Session location source (GPS-only default vs opt-in Google fused) ---

    /** Location source for the wardrive session; GPS_PROVIDER_ONLY is the default. */
    val locationSourceFlow: Flow<LocationSource> =
        dataStore.data.map { prefs ->
            when (prefs[KEY_LOCATION_SOURCE]) {
                "fused_opt_in" -> LocationSource.FUSED_OPT_IN
                else -> LocationSource.GPS_PROVIDER_ONLY
            }
        }

    suspend fun locationSource(): LocationSource = locationSourceFlow.first()

    suspend fun setLocationSource(source: LocationSource) {
        dataStore.edit { prefs ->
            prefs[KEY_LOCATION_SOURCE] =
                when (source) {
                    LocationSource.FUSED_OPT_IN -> "fused_opt_in"
                    LocationSource.GPS_PROVIDER_ONLY -> "gps_provider_only"
                }
        }
    }

    // --- Sniffer channel plan (dwell + band pools; ChannelPlanSync or legacy PolicySync tail) ---

    data class ChannelPlan(
        val revision: Int = 0,
        val dwellMs: Int,
        val include24Ghz: Boolean = true,
        val include5Ghz: Boolean = true,
        val includeDfs: Boolean = true,
        val bandMode: String = BAND_MODE_AUTO,
        val channels24: List<Int> = DEFAULT_CHANNELS_24,
        val channels5: List<Int> = DEFAULT_CHANNELS_5,
        val channels5Dfs: List<Int> = DEFAULT_CHANNELS_5_DFS,
    ) {
        /** Flat hop list for legacy PolicySync when gateway lacks ChannelPlanSync. */
        fun legacyFlatChannels(): List<Int> {
            val out = mutableListOf<Int>()
            if (include24Ghz) out.addAll(channels24)
            if (include5Ghz) out.addAll(channels5)
            if (include5Ghz && includeDfs) out.addAll(channels5Dfs)
            return out.distinct().sorted()
        }

        fun isManualBandMode(): Boolean = bandMode == BAND_MODE_MANUAL
    }

    val channelPlanFlow: Flow<ChannelPlan> =
        dataStore.data.map { prefs -> readChannelPlan(prefs) }

    suspend fun currentChannelPlan(): ChannelPlan = channelPlanFlow.first()

    suspend fun saveChannelPlan(plan: ChannelPlan) {
        dataStore.edit { prefs ->
            prefs[KEY_CHANNEL_PLAN_REVISION] = plan.revision.coerceAtLeast(0)
            prefs[KEY_CHANNEL_DWELL_MS] = plan.dwellMs.coerceIn(MIN_DWELL_MS, MAX_DWELL_MS)
            prefs[KEY_CHANNEL_INCLUDE_24] = plan.include24Ghz
            prefs[KEY_CHANNEL_INCLUDE_5] = plan.include5Ghz
            prefs[KEY_CHANNEL_INCLUDE_DFS] = plan.includeDfs
            prefs[KEY_CHANNEL_BAND_MODE] = plan.bandMode
            prefs[KEY_CHANNELS_24] = plan.channels24.distinct().sorted().joinToString(",")
            prefs[KEY_CHANNELS_5] = plan.channels5.distinct().sorted().joinToString(",")
            prefs[KEY_CHANNELS_5_DFS] = plan.channels5Dfs.distinct().sorted().joinToString(",")
        }
    }

    suspend fun resetChannelPlanDefaults() {
        saveChannelPlan(defaultChannelPlan(revision = currentChannelPlan().revision + 1))
    }

    suspend fun incrementRevision(): Int {
        var next = 0
        dataStore.edit { prefs ->
            next = (prefs[KEY_CHANNEL_PLAN_REVISION] ?: 0) + 1
            prefs[KEY_CHANNEL_PLAN_REVISION] = next
        }
        return next
    }

    private fun readChannelPlan(prefs: Preferences): ChannelPlan {
        val dwell = prefs[KEY_CHANNEL_DWELL_MS] ?: DEFAULT_DWELL_MS
        val revision = prefs[KEY_CHANNEL_PLAN_REVISION] ?: 0
        val include24 = prefs[KEY_CHANNEL_INCLUDE_24] ?: true
        val include5 = prefs[KEY_CHANNEL_INCLUDE_5] ?: true
        val includeDfs = prefs[KEY_CHANNEL_INCLUDE_DFS] ?: true
        val bandMode = prefs[KEY_CHANNEL_BAND_MODE] ?: BAND_MODE_AUTO
        val channels24 =
            parseChannelCsv(prefs[KEY_CHANNELS_24])
                ?: parseChannelCsv(prefs[KEY_CHANNEL_LIST])
                ?: DEFAULT_CHANNELS_24
        val channels5 = parseChannelCsv(prefs[KEY_CHANNELS_5]) ?: DEFAULT_CHANNELS_5
        val channels5Dfs = parseChannelCsv(prefs[KEY_CHANNELS_5_DFS]) ?: DEFAULT_CHANNELS_5_DFS
        return ChannelPlan(
            revision = revision,
            dwellMs = dwell,
            include24Ghz = include24,
            include5Ghz = include5,
            includeDfs = includeDfs,
            bandMode = if (bandMode == BAND_MODE_MANUAL) BAND_MODE_MANUAL else BAND_MODE_AUTO,
            channels24 = channels24,
            channels5 = channels5,
            channels5Dfs = channels5Dfs,
        )
    }

    private fun defaultChannelPlan(revision: Int = 0): ChannelPlan =
        ChannelPlan(
            revision = revision,
            dwellMs = DEFAULT_DWELL_MS,
            include24Ghz = true,
            include5Ghz = true,
            includeDfs = true,
            bandMode = BAND_MODE_AUTO,
            channels24 = DEFAULT_CHANNELS_24,
            channels5 = DEFAULT_CHANNELS_5,
            channels5Dfs = DEFAULT_CHANNELS_5_DFS,
        )

    // --- Upload credentials (WiGLE + wdgwars) ---

    val wigleApiNameFlow: Flow<String> =
        dataStore.data.map { it[KEY_WIGLE_API_NAME].orEmpty() }

    val wigleApiTokenFlow: Flow<String> =
        dataStore.data.map { it[KEY_WIGLE_API_TOKEN].orEmpty() }

    val wdgwarsApiKeyFlow: Flow<String> =
        dataStore.data.map { it[KEY_WDGWARS_API_KEY].orEmpty() }

    suspend fun setWigleApiName(value: String) {
        dataStore.edit { it[KEY_WIGLE_API_NAME] = value }
    }

    suspend fun setWigleApiToken(value: String) {
        dataStore.edit { it[KEY_WIGLE_API_TOKEN] = value }
    }

    suspend fun setWdgwarsApiKey(value: String) {
        dataStore.edit { it[KEY_WDGWARS_API_KEY] = value }
    }

    // --- WiGLE filter denylists (newline-separated text areas) ---

    val denylistBssidsTextFlow: Flow<String> =
        dataStore.data.map { it[KEY_DENYLIST_BSSIDS_TEXT].orEmpty() }

    val denylistSsidsTextFlow: Flow<String> =
        dataStore.data.map { it[KEY_DENYLIST_SSIDS_TEXT].orEmpty() }

    suspend fun setDenylistBssidsText(value: String) {
        dataStore.edit { it[KEY_DENYLIST_BSSIDS_TEXT] = value }
    }

    suspend fun setDenylistSsidsText(value: String) {
        dataStore.edit { it[KEY_DENYLIST_SSIDS_TEXT] = value }
    }

    // --- Detection methods (flock disable masks + local MAC ignore + SSID watch) ---

    val detectionDisableWifiMaskFlow: Flow<Int> =
        dataStore.data.map {
            it[KEY_DETECTION_DISABLE_WIFI_MASK]
                ?: it[KEY_LEGACY_FLOCK_DISABLE_WIFI_MASK]
                ?: DEFAULT_DETECTION_DISABLE_WIFI_MASK
        }

    val detectionDisableBleMaskFlow: Flow<Int> =
        dataStore.data.map {
            it[KEY_DETECTION_DISABLE_BLE_MASK]
                ?: it[KEY_LEGACY_FLOCK_DISABLE_BLE_MASK]
                ?: DEFAULT_DETECTION_DISABLE_BLE_MASK
        }

    val detectionMacIgnoreTextFlow: Flow<String> =
        dataStore.data.map {
            (it[KEY_DETECTION_MAC_IGNORE_TEXT] ?: it[KEY_LEGACY_FLOCK_MAC_IGNORE_TEXT]).orEmpty()
        }

    val ssidWatchTextFlow: Flow<String> =
        dataStore.data.map { it[KEY_SSID_WATCH_TEXT].orEmpty() }

    suspend fun setDetectionDisableWifiMask(mask: Int) {
        dataStore.edit { it[KEY_DETECTION_DISABLE_WIFI_MASK] = mask }
    }

    suspend fun setDetectionDisableBleMask(mask: Int) {
        dataStore.edit { it[KEY_DETECTION_DISABLE_BLE_MASK] = mask }
    }

    /** BLE-role sniffers use active LE scan (SCAN_REQ) when true; passive when false (default). */
    val bleActiveScanFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_BLE_ACTIVE_SCAN] == true }

    suspend fun setBleActiveScan(enabled: Boolean) {
        dataStore.edit { it[KEY_BLE_ACTIVE_SCAN] = enabled }
    }

    suspend fun setDetectionMacIgnoreText(value: String) {
        dataStore.edit { it[KEY_DETECTION_MAC_IGNORE_TEXT] = value }
    }

    suspend fun setSsidWatchText(value: String) {
        dataStore.edit { it[KEY_SSID_WATCH_TEXT] = value }
    }

    // --- Home zone (CSV enrichment gate; circle around lat/lon) ---

    val homeZoneEnabledFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_HOME_ZONE_ENABLED] == true }

    val homeZoneLatFlow: Flow<Double?> =
        dataStore.data.map { it[KEY_HOME_ZONE_LAT] }

    val homeZoneLonFlow: Flow<Double?> =
        dataStore.data.map { it[KEY_HOME_ZONE_LON] }

    val homeZoneRadiusMFlow: Flow<Double> =
        dataStore.data.map { it[KEY_HOME_ZONE_RADIUS_M] ?: DEFAULT_HOME_ZONE_RADIUS_M }

    suspend fun saveHomeZone(lat: Double, lon: Double, radiusM: Double) {
        dataStore.edit { prefs ->
            prefs[KEY_HOME_ZONE_ENABLED] = true
            prefs[KEY_HOME_ZONE_LAT] = lat
            prefs[KEY_HOME_ZONE_LON] = lon
            prefs[KEY_HOME_ZONE_RADIUS_M] = radiusM
        }
    }

    suspend fun clearHomeZone() {
        dataStore.edit { prefs ->
            prefs[KEY_HOME_ZONE_ENABLED] = false
            prefs.remove(KEY_HOME_ZONE_LAT)
            prefs.remove(KEY_HOME_ZONE_LON)
        }
    }

    // --- Detection notification prefs (master + sound + vibration) ---

    val detectionAlertsEnabledFlow: Flow<Boolean> =
        dataStore.data.map {
            it[KEY_DETECTION_ALERTS_ENABLED] ?: it[KEY_LEGACY_FLOCK_ALERTS_ENABLED] ?: true
        }

    val soundDetectionFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_SOUND_DETECTION] ?: it[KEY_LEGACY_SOUND_FLOCK] ?: true }

    val vibrateDetectionFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_VIBRATE_DETECTION] ?: it[KEY_LEGACY_VIBRATE_FLOCK] ?: true }

    suspend fun setDetectionAlertsEnabled(enabled: Boolean) {
        dataStore.edit { it[KEY_DETECTION_ALERTS_ENABLED] = enabled }
    }

    suspend fun setSoundDetection(enabled: Boolean) {
        dataStore.edit { it[KEY_SOUND_DETECTION] = enabled }
    }

    suspend fun setVibrateDetection(enabled: Boolean) {
        dataStore.edit { it[KEY_VIBRATE_DETECTION] = enabled }
    }

    // --- Geofence transition alerts (enter/leave square during wardrive) ---

    val geofenceAlertsEnabledFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_GEOFENCE_ALERTS_ENABLED] ?: true }

    suspend fun setGeofenceAlertsEnabled(enabled: Boolean) {
        dataStore.edit { it[KEY_GEOFENCE_ALERTS_ENABLED] = enabled }
    }

    suspend fun purgeLegacyGeofenceDetectionsIfNeeded(db: net.wardriver.mobile.db.WardriverDatabase) {
        if (dataStore.data.first()[KEY_GEOFENCE_DETECTION_PURGED_V1] == true) return
        db.detectionDao().deleteGeofenceTransitions()
        dataStore.edit { it[KEY_GEOFENCE_DETECTION_PURGED_V1] = true }
    }

    val detectionDeliveryModeFlow: Flow<AlertDeliveryMode> =
        dataStore.data.map { prefs ->
            when (prefs[KEY_DETECTION_DELIVERY_MODE] ?: prefs[KEY_LEGACY_FLOCK_DELIVERY_MODE]) {
                "full" -> AlertDeliveryMode.Full
                "quiet" -> AlertDeliveryMode.Quiet
                else -> AlertDeliveryMode.InAppWhenForeground
            }
        }

    val detectionNotifyCooldownMsFlow: Flow<Long> =
        dataStore.data.map { prefs ->
            (prefs[KEY_DETECTION_NOTIFY_COOLDOWN_MS]
                    ?: prefs[KEY_LEGACY_FLOCK_NOTIFY_COOLDOWN_MS]
                    ?: DEFAULT_DETECTION_NOTIFY_COOLDOWN_MS).toLong()
        }

    suspend fun setDetectionDeliveryMode(mode: AlertDeliveryMode) {
        dataStore.edit { prefs ->
            prefs[KEY_DETECTION_DELIVERY_MODE] =
                when (mode) {
                    AlertDeliveryMode.InAppWhenForeground -> "in_app_when_foreground"
                    AlertDeliveryMode.Full -> "full"
                    AlertDeliveryMode.Quiet -> "quiet"
                }
        }
    }

    // --- Network upload policy ---

    val uploadOnCellularFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_UPLOAD_ON_CELLULAR] == true }

    suspend fun setUploadOnCellular(enabled: Boolean) {
        dataStore.edit { it[KEY_UPLOAD_ON_CELLULAR] = enabled }
    }

    // --- Phone-sourced cell tower logging ---

    val cellTowersEnabledFlow: Flow<Boolean> =
        dataStore.data.map { it[KEY_CELL_TOWERS_ENABLED] != false }

    suspend fun setCellTowersEnabled(enabled: Boolean) {
        dataStore.edit { it[KEY_CELL_TOWERS_ENABLED] = enabled }
    }

    suspend fun isCellTowersEnabled(): Boolean = cellTowersEnabledFlow.first()

    /** One-shot read for workers and policy assembly. */
    suspend fun snapshot(): PrefsSnapshot {
        val prefs = dataStore.data.first()
        return PrefsSnapshot(
            geofence =
                if (prefs[KEY_GEOFENCE_ENABLED] == true) {
                    val latMin = prefs[KEY_GEOFENCE_LAT_MIN]
                    val latMax = prefs[KEY_GEOFENCE_LAT_MAX]
                    val lonMin = prefs[KEY_GEOFENCE_LON_MIN]
                    val lonMax = prefs[KEY_GEOFENCE_LON_MAX]
                    if (latMin != null && latMax != null && lonMin != null && lonMax != null &&
                        latMin < latMax && lonMin < lonMax
                    ) {
                        GeofenceBounds(latMin, latMax, lonMin, lonMax)
                    } else {
                        null
                    }
                } else {
                    null
                },
            wigleApiName = prefs[KEY_WIGLE_API_NAME].orEmpty(),
            wigleApiToken = prefs[KEY_WIGLE_API_TOKEN].orEmpty(),
            wdgwarsApiKey = prefs[KEY_WDGWARS_API_KEY].orEmpty(),
            denylistBssidsText = prefs[KEY_DENYLIST_BSSIDS_TEXT].orEmpty(),
            denylistSsidsText = prefs[KEY_DENYLIST_SSIDS_TEXT].orEmpty(),
            detectionDisableWifiMask =
                prefs[KEY_DETECTION_DISABLE_WIFI_MASK]
                    ?: prefs[KEY_LEGACY_FLOCK_DISABLE_WIFI_MASK]
                    ?: DEFAULT_DETECTION_DISABLE_WIFI_MASK,
            detectionDisableBleMask =
                prefs[KEY_DETECTION_DISABLE_BLE_MASK]
                    ?: prefs[KEY_LEGACY_FLOCK_DISABLE_BLE_MASK]
                    ?: DEFAULT_DETECTION_DISABLE_BLE_MASK,
            bleActiveScan = prefs[KEY_BLE_ACTIVE_SCAN] == true,
            detectionMacIgnoreText =
                (prefs[KEY_DETECTION_MAC_IGNORE_TEXT] ?: prefs[KEY_LEGACY_FLOCK_MAC_IGNORE_TEXT]).orEmpty(),
            ssidWatchText = prefs[KEY_SSID_WATCH_TEXT].orEmpty(),
            homeZoneEnabled = prefs[KEY_HOME_ZONE_ENABLED] == true,
            homeZoneLat = prefs[KEY_HOME_ZONE_LAT],
            homeZoneLon = prefs[KEY_HOME_ZONE_LON],
            homeZoneRadiusM = prefs[KEY_HOME_ZONE_RADIUS_M] ?: DEFAULT_HOME_ZONE_RADIUS_M,
            uploadOnCellular = prefs[KEY_UPLOAD_ON_CELLULAR] == true,
            scanAllowed = prefs[KEY_SCAN_ALLOWED] != false,
            cellTowersEnabled = prefs[KEY_CELL_TOWERS_ENABLED] != false,
            channelPlan = readChannelPlan(prefs),
        )
    }

    /**
     * One-time copy of the legacy `flock_*` DataStore keys into the `detection_*` keys so old
     * installs keep their settings after the Flock → Detection rename. The legacy keys remain
     * readable as a fallback (see the flows above); this copy makes the new keys authoritative.
     */
    suspend fun migrateLegacyDetectionKeysIfNeeded() {
        dataStore.edit { prefs ->
            // Int keys.
            copyIfAbsentInt(prefs, KEY_DETECTION_DISABLE_WIFI_MASK, KEY_LEGACY_FLOCK_DISABLE_WIFI_MASK)
            copyIfAbsentInt(prefs, KEY_DETECTION_DISABLE_BLE_MASK, KEY_LEGACY_FLOCK_DISABLE_BLE_MASK)
            copyIfAbsentInt(prefs, KEY_DETECTION_NOTIFY_COOLDOWN_MS, KEY_LEGACY_FLOCK_NOTIFY_COOLDOWN_MS)
            // Boolean keys.
            copyIfAbsentBoolean(prefs, KEY_DETECTION_ALERTS_ENABLED, KEY_LEGACY_FLOCK_ALERTS_ENABLED)
            copyIfAbsentBoolean(prefs, KEY_SOUND_DETECTION, KEY_LEGACY_SOUND_FLOCK)
            copyIfAbsentBoolean(prefs, KEY_VIBRATE_DETECTION, KEY_LEGACY_VIBRATE_FLOCK)
            copyIfAbsentBoolean(prefs, KEY_MAP_FILTER_ALERT_DETECTION_WIFI, KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_WIFI)
            copyIfAbsentBoolean(prefs, KEY_MAP_FILTER_ALERT_DETECTION_BLE, KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_BLE)
            copyIfAbsentBoolean(prefs, KEY_MAP_FILTER_DETECTION_WIFI, KEY_LEGACY_MAP_FILTER_FLOCK_WIFI)
            copyIfAbsentBoolean(prefs, KEY_MAP_FILTER_DETECTION_BLE, KEY_LEGACY_MAP_FILTER_FLOCK_BLE)
            // String keys.
            copyIfAbsentString(prefs, KEY_DETECTION_MAC_IGNORE_TEXT, KEY_LEGACY_FLOCK_MAC_IGNORE_TEXT)
            copyIfAbsentString(prefs, KEY_DETECTION_DELIVERY_MODE, KEY_LEGACY_FLOCK_DELIVERY_MODE)
        }
    }

    private fun copyIfAbsentInt(prefs: MutablePreferences, newKey: Preferences.Key<Int>, legacyKey: Preferences.Key<Int>) {
        if (newKey !in prefs && legacyKey in prefs) prefs[newKey] = prefs[legacyKey]!!
    }

    private fun copyIfAbsentBoolean(prefs: MutablePreferences, newKey: Preferences.Key<Boolean>, legacyKey: Preferences.Key<Boolean>) {
        if (newKey !in prefs && legacyKey in prefs) prefs[newKey] = prefs[legacyKey]!!
    }

    private fun copyIfAbsentString(prefs: MutablePreferences, newKey: Preferences.Key<String>, legacyKey: Preferences.Key<String>) {
        if (newKey !in prefs && legacyKey in prefs) prefs[newKey] = prefs[legacyKey]!!
    }

    /**
     * Copy legacy SharedPreferences upload creds into DataStore once (Phase 1 → Phase 3 bridge).
     */
    suspend fun migrateLegacyUploadPrefsIfNeeded() {
        val legacy =
            context.getSharedPreferences(LEGACY_UPLOAD_PREFS, Context.MODE_PRIVATE)
        val legacyName = legacy.getString(LEGACY_KEY_WIGLE_NAME, null)
        val legacyToken = legacy.getString(LEGACY_KEY_WIGLE_TOKEN, null)
        val legacyWdg = legacy.getString(LEGACY_KEY_WDG_KEY, null)
        if (legacyName.isNullOrBlank() && legacyToken.isNullOrBlank() && legacyWdg.isNullOrBlank()) {
            return
        }
        dataStore.edit { prefs ->
            if (prefs[KEY_WIGLE_API_NAME].isNullOrBlank() && !legacyName.isNullOrBlank()) {
                prefs[KEY_WIGLE_API_NAME] = legacyName
            }
            if (prefs[KEY_WIGLE_API_TOKEN].isNullOrBlank() && !legacyToken.isNullOrBlank()) {
                prefs[KEY_WIGLE_API_TOKEN] = legacyToken
            }
            if (prefs[KEY_WDGWARS_API_KEY].isNullOrBlank() && !legacyWdg.isNullOrBlank()) {
                prefs[KEY_WDGWARS_API_KEY] = legacyWdg
            }
        }
    }

    data class PrefsSnapshot(
        val geofence: GeofenceBounds?,
        val wigleApiName: String,
        val wigleApiToken: String,
        val wdgwarsApiKey: String,
        val denylistBssidsText: String,
        val denylistSsidsText: String,
        val detectionDisableWifiMask: Int,
        val detectionDisableBleMask: Int,
        val bleActiveScan: Boolean,
        val detectionMacIgnoreText: String,
        val ssidWatchText: String,
        val homeZoneEnabled: Boolean,
        val homeZoneLat: Double?,
        val homeZoneLon: Double?,
        val homeZoneRadiusM: Double,
        val uploadOnCellular: Boolean,
        val scanAllowed: Boolean,
        val cellTowersEnabled: Boolean,
        val channelPlan: ChannelPlan,
    )

    companion object {
        // Geofence map bounds (axis-aligned rectangle).
        private val KEY_GEOFENCE_ENABLED = booleanPreferencesKey("geofence_enabled")
        private val KEY_GEOFENCE_LAT_MIN = doublePreferencesKey("geofence_lat_min")
        private val KEY_GEOFENCE_LAT_MAX = doublePreferencesKey("geofence_lat_max")
        private val KEY_GEOFENCE_LON_MIN = doublePreferencesKey("geofence_lon_min")
        private val KEY_GEOFENCE_LON_MAX = doublePreferencesKey("geofence_lon_max")

        // Map layer filters and data scope (detection keys; legacy flock_ keys readable).
        private val KEY_MAP_DATA_SCOPE = stringPreferencesKey("map_data_scope")
        private val KEY_MAP_FILTER_WIFI = booleanPreferencesKey("map_filter_wifi")
        private val KEY_MAP_FILTER_BLE = booleanPreferencesKey("map_filter_ble")
        private val KEY_MAP_FILTER_PROBE = booleanPreferencesKey("map_filter_probe")
        private val KEY_MAP_FILTER_DETECTION_WIFI = booleanPreferencesKey("map_filter_detection_wifi")
        private val KEY_MAP_FILTER_DETECTION_BLE = booleanPreferencesKey("map_filter_detection_ble")
        private val KEY_LEGACY_MAP_FILTER_FLOCK_WIFI = booleanPreferencesKey("map_filter_flock_wifi")
        private val KEY_LEGACY_MAP_FILTER_FLOCK_BLE = booleanPreferencesKey("map_filter_flock_ble")
        private val KEY_MAP_FILTER_ALERTS = booleanPreferencesKey("map_filter_alerts")
        private val KEY_MAP_FILTER_SSID_WATCH = booleanPreferencesKey("map_filter_ssid_watch")
        private val KEY_MAP_FILTER_REMOTE_ID = booleanPreferencesKey("map_filter_remote_id")
        // Per-type alert visibility (detection keys; old flock_ keys migrated on read).
        private val KEY_MAP_FILTER_ALERT_DETECTION_WIFI = booleanPreferencesKey("map_filter_alert_detection_wifi")
        private val KEY_MAP_FILTER_ALERT_DETECTION_BLE = booleanPreferencesKey("map_filter_alert_detection_ble")
        private val KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_WIFI = booleanPreferencesKey("map_filter_alert_flock_wifi")
        private val KEY_LEGACY_MAP_FILTER_ALERT_FLOCK_BLE = booleanPreferencesKey("map_filter_alert_flock_ble")
        private val KEY_MAP_FILTER_ALERT_SSID_WATCH = booleanPreferencesKey("map_filter_alert_ssid_watch")
        private val KEY_MAP_FILTER_ALERT_REMOTE_ID = booleanPreferencesKey("map_filter_alert_remote_id")
        private val KEY_MAP_FILTER_ALERT_AXON = booleanPreferencesKey("map_filter_alert_axon")
        private val KEY_MAP_FILTER_ALERT_IPRO = booleanPreferencesKey("map_filter_alert_ipro")
        private val KEY_MAP_COVERAGE_SHADING = booleanPreferencesKey("map_coverage_shading")
        private val KEY_MAP_LAYER_PANEL_EXPANDED = booleanPreferencesKey("map_layer_panel_expanded")
        private val KEY_MAP_BASEMAP = stringPreferencesKey("map_basemap")

        // WiGLE / wdgwars API credentials.
        private val KEY_WIGLE_API_NAME = stringPreferencesKey("wigle_api_name")
        private val KEY_WIGLE_API_TOKEN = stringPreferencesKey("wigle_api_token")
        private val KEY_WDGWARS_API_KEY = stringPreferencesKey("wdgwars_api_key")

        // WiGLE CSV exclusion lists (raw newline text).
        private val KEY_DENYLIST_BSSIDS_TEXT = stringPreferencesKey("denylist_bssids_text")
        private val KEY_DENYLIST_SSIDS_TEXT = stringPreferencesKey("denylist_ssids_text")

        // Detection disable bitmasks (bit set = method disabled).
        private val KEY_DETECTION_DISABLE_WIFI_MASK = intPreferencesKey("detection_disable_wifi_mask")
        private val KEY_DETECTION_DISABLE_BLE_MASK = intPreferencesKey("detection_disable_ble_mask")
        private val KEY_LEGACY_FLOCK_DISABLE_WIFI_MASK = intPreferencesKey("flock_disable_wifi_mask")
        private val KEY_LEGACY_FLOCK_DISABLE_BLE_MASK = intPreferencesKey("flock_disable_ble_mask")
        private val KEY_BLE_ACTIVE_SCAN = booleanPreferencesKey("ble_active_scan")

        // Phone-local detection MAC ignore list (not on policy_sync wire yet).
        private val KEY_DETECTION_MAC_IGNORE_TEXT = stringPreferencesKey("detection_mac_ignore_text")
        private val KEY_LEGACY_FLOCK_MAC_IGNORE_TEXT = stringPreferencesKey("flock_mac_ignore_text")

        // SSID watch list (newline text; pushed in policy_sync).
        private val KEY_SSID_WATCH_TEXT = stringPreferencesKey("ssid_watch_text")

        // Home zone circle for enrichment CSV gate.
        private val KEY_HOME_ZONE_ENABLED = booleanPreferencesKey("home_zone_enabled")
        private val KEY_HOME_ZONE_LAT = doublePreferencesKey("home_zone_lat")
        private val KEY_HOME_ZONE_LON = doublePreferencesKey("home_zone_lon")
        private val KEY_HOME_ZONE_RADIUS_M = doublePreferencesKey("home_zone_radius_m")

        // Detection alert notification toggles (phone-local).
        private val KEY_DETECTION_ALERTS_ENABLED = booleanPreferencesKey("detection_alerts_enabled")
        private val KEY_LEGACY_FLOCK_ALERTS_ENABLED = booleanPreferencesKey("flock_alerts_enabled")
        private val KEY_SOUND_DETECTION = booleanPreferencesKey("sound_detection")
        private val KEY_LEGACY_SOUND_FLOCK = booleanPreferencesKey("sound_flock")
        private val KEY_VIBRATE_DETECTION = booleanPreferencesKey("vibrate_detection")
        private val KEY_LEGACY_VIBRATE_FLOCK = booleanPreferencesKey("vibrate_flock")
        private val KEY_GEOFENCE_ALERTS_ENABLED = booleanPreferencesKey("geofence_alerts_enabled")
        private val KEY_GEOFENCE_DETECTION_PURGED_V1 = booleanPreferencesKey("geofence_detection_purged_v1")
        private val KEY_DETECTION_DELIVERY_MODE = stringPreferencesKey("detection_delivery_mode")
        private val KEY_LEGACY_FLOCK_DELIVERY_MODE = stringPreferencesKey("flock_delivery_mode")
        private val KEY_DETECTION_NOTIFY_COOLDOWN_MS = intPreferencesKey("detection_notify_cooldown_ms")
        private val KEY_LEGACY_FLOCK_NOTIFY_COOLDOWN_MS = intPreferencesKey("flock_notify_cooldown_ms")

        // Allow uploads on cellular data.
        private val KEY_UPLOAD_ON_CELLULAR = booleanPreferencesKey("upload_on_cellular")

        // Log cell towers from this phone (TelephonyManager; gateway sniffers unchanged).
        private val KEY_CELL_TOWERS_ENABLED = booleanPreferencesKey("cell_towers_enabled")

        // Gateway SoftAP join credentials (fleet default wolfPACK / deflock123).
        private val KEY_GATEWAY_SOFTAP_SSID = stringPreferencesKey("gateway_softap_ssid")
        private val KEY_GATEWAY_SOFTAP_PASS = stringPreferencesKey("gateway_softap_pass")
        const val DEFAULT_GATEWAY_SOFTAP_SSID = "wolfPACK"
        const val DEFAULT_GATEWAY_SOFTAP_PASS = "deflock123"

        // Legacy NAN peer id (unused after SoftAP migration).
        private val KEY_PREFERRED_GATEWAY_PEER_ID = stringPreferencesKey("preferred_gateway_peer_id")

        // Fleet scan gate (false = Pause / management mode).
        private val KEY_SCAN_ALLOWED = booleanPreferencesKey("scan_allowed")

        // Foreground wardrive session resume after process death.
        private val KEY_WARDRIVE_ACTIVE = booleanPreferencesKey("wardrive_active")
        private val KEY_ACTIVE_WARDRIVE_SESSION_ID = longPreferencesKey("active_wardrive_session_id")
        private val KEY_ACTIVE_SESSION_GATEWAY_MAC = stringPreferencesKey("active_session_gateway_mac")

        // Session location source (gps_provider_only default; fused_opt_in when user opts in).
        private val KEY_LOCATION_SOURCE = stringPreferencesKey("location_source")

        // Channel plan persisted for ChannelPlanSync / legacy PolicySync dwell + hop list.
        private val KEY_CHANNEL_PLAN_REVISION = intPreferencesKey("channel_plan_revision")
        private val KEY_CHANNEL_DWELL_MS = intPreferencesKey("channel_dwell_ms")
        private val KEY_CHANNEL_INCLUDE_24 = booleanPreferencesKey("channel_include_24")
        private val KEY_CHANNEL_INCLUDE_5 = booleanPreferencesKey("channel_include_5")
        private val KEY_CHANNEL_INCLUDE_DFS = booleanPreferencesKey("channel_include_dfs")
        private val KEY_CHANNEL_BAND_MODE = stringPreferencesKey("channel_band_mode")
        private val KEY_CHANNELS_24 = stringPreferencesKey("channels_24")
        private val KEY_CHANNELS_5 = stringPreferencesKey("channels_5")
        private val KEY_CHANNELS_5_DFS = stringPreferencesKey("channels_5_dfs")
        /** Legacy MVP key; migrated into [KEY_CHANNELS_24] on read when unset. */
        private val KEY_CHANNEL_LIST = stringPreferencesKey("channel_list")

        const val BAND_MODE_AUTO = "auto"
        const val BAND_MODE_MANUAL = "manual"
        const val MIN_DWELL_MS = 50
        const val MAX_DWELL_MS = 10_000
        const val DEFAULT_DWELL_MS = 200
        val DEFAULT_CHANNELS_24: List<Int> = (1..11).toList()
        val DEFAULT_CHANNELS_5: List<Int> =
            listOf(36, 40, 44, 48, 149, 153, 157, 161, 165, 169, 173, 177)
        val DEFAULT_CHANNELS_5_DFS: List<Int> =
            listOf(52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144)

        /** @deprecated Use [DEFAULT_CHANNELS_24] or [ChannelPlan.legacyFlatChannels]. */
        @Deprecated("Use DEFAULT_CHANNELS_24")
        val DEFAULT_CHANNELS: List<Int> = listOf(1, 6, 11)

        private fun parseChannelCsv(raw: String?): List<Int>? {
            if (raw.isNullOrBlank()) return null
            return raw
                .split(',')
                .mapNotNull { it.trim().toIntOrNull() }
                .filter { it in 1..177 }
                .distinct()
                .sorted()
                .ifEmpty { null }
        }

        // wolfPACK defaults: WiFi method 1 off; BLE methods 32-34 off; Axon/i-PRO (4/5/35/36) on.
        const val DEFAULT_DETECTION_DISABLE_WIFI_MASK = 0x1
        const val DEFAULT_DETECTION_DISABLE_BLE_MASK = 0x7
        const val DEFAULT_HOME_ZONE_RADIUS_M = 1000.0
        const val DEFAULT_DETECTION_NOTIFY_COOLDOWN_MS = 30_000

        private const val LEGACY_UPLOAD_PREFS = "wardriver_upload"
        private const val LEGACY_KEY_WIGLE_NAME = "wigle_api_name"
        private const val LEGACY_KEY_WIGLE_TOKEN = "wigle_api_token"
        private const val LEGACY_KEY_WDG_KEY = "wdgwars_api_key"

        /** Parse newline-separated MAC / SSID lines for policy_sync lists. */
        fun parseLineList(text: String): List<String> =
            text
                .lineSequence()
                .map { it.trim() }
                .filter { it.isNotEmpty() }
                .toList()

        /** WiFi method `m` enabled when disable-bit `(m-1)` is clear. */
        fun isWifiMethodEnabled(mask: Int, method: Int): Boolean =
            (mask and (1 shl (method - 1))) == 0

        /** BLE method `m` enabled when disable-bit `(m-32)` is clear. */
        fun isBleMethodEnabled(mask: Int, method: Int): Boolean =
            (mask and (1 shl (method - 32))) == 0

        fun setWifiMethodEnabled(mask: Int, method: Int, enabled: Boolean): Int {
            val bit = 1 shl (method - 1)
            return if (enabled) mask and bit.inv() else mask or bit
        }

        fun setBleMethodEnabled(mask: Int, method: Int, enabled: Boolean): Int {
            val bit = 1 shl (method - 32)
            return if (enabled) mask and bit.inv() else mask or bit
        }
    }
}
