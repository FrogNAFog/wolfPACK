package net.wardriver.mobile.ui.nav

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import net.wardriver.mobile.ui.MapScreen
import net.wardriver.mobile.ui.dashboard.DashboardScreen
import net.wardriver.mobile.ui.settings.SettingsHubScreen
import net.wardriver.mobile.ui.settings.capture.CaptureScreen
import net.wardriver.mobile.ui.settings.detections.DetectionsScreen
import net.wardriver.mobile.ui.settings.filters.WigleFiltersScreen
import net.wardriver.mobile.ui.settings.homezone.HomeZoneScreen
import net.wardriver.mobile.ui.settings.location.LocationSourceScreen
import net.wardriver.mobile.ui.settings.network.NetworkScreen
import net.wardriver.mobile.ui.settings.notifications.NotificationsScreen
import net.wardriver.mobile.ui.settings.uploads.UploadsScreen
import net.wardriver.mobile.ui.settings.uploads.WigleQrScanScreen
import net.wardriver.mobile.ui.settings.map.MapSettingsScreen
import net.wardriver.mobile.ui.settings.gateway.GatewayScreen
import net.wardriver.mobile.ui.settings.channelplan.ChannelPlanScreen
import net.wardriver.mobile.ui.sniffers.SniffersScreen

/** Nested settings routes under the settings tab. */
object SettingsRoutes {
    const val Uploads = "settings/uploads"
    const val WigleQrScan = "settings/uploads/wigle-qr-scan"
    const val Filters = "settings/filters"
    const val Detections = "settings/detections"
    const val HomeZone = "settings/home_zone"
    const val Location = "settings/location"
    const val Notifications = "settings/notifications"
    const val Network = "settings/network"
    const val Capture = "settings/capture"
    const val Gateway = "settings/gateway"
    const val ChannelPlan = "settings/channel_plan"
    const val Map = "settings/map"
}

@Composable
fun AppNavHost(
    navController: NavHostController,
    modifier: Modifier = Modifier,
) {
    NavHost(
        navController = navController,
        startDestination = AppDestinations.Map.route,
        modifier = modifier,
    ) {
        composable(AppDestinations.Dashboard.route) {
            DashboardScreen(
                modifier = Modifier.fillMaxSize(),
                onNavigateGatewaySettings = { navController.navigate(SettingsRoutes.Gateway) },
            )
        }
        composable(AppDestinations.Sniffers.route) {
            SniffersScreen(
                modifier = Modifier.fillMaxSize(),
                onNavigateGatewaySettings = { navController.navigate(SettingsRoutes.Gateway) },
            )
        }
        composable(AppDestinations.Map.route) {
            MapScreen(
                modifier = Modifier.fillMaxSize(),
            )
        }
        composable(AppDestinations.Settings.route) {
            SettingsHubScreen(
                onNavigateUploads = { navController.navigate(SettingsRoutes.Uploads) },
                onNavigateFilters = { navController.navigate(SettingsRoutes.Filters) },
                onNavigateDetections = { navController.navigate(SettingsRoutes.Detections) },
                onNavigateHomeZone = { navController.navigate(SettingsRoutes.HomeZone) },
                onNavigateLocation = { navController.navigate(SettingsRoutes.Location) },
                onNavigateNotifications = { navController.navigate(SettingsRoutes.Notifications) },
                onNavigateNetwork = { navController.navigate(SettingsRoutes.Network) },
                onNavigateCapture = { navController.navigate(SettingsRoutes.Capture) },
                onNavigateGateway = { navController.navigate(SettingsRoutes.Gateway) },
                onNavigateChannelPlan = { navController.navigate(SettingsRoutes.ChannelPlan) },
                onNavigateMap = { navController.navigate(SettingsRoutes.Map) },
            )
        }
        composable(SettingsRoutes.Uploads) { entry ->
            val qrImported by entry.savedStateHandle
                .getStateFlow("wigle_qr_imported", false)
                .collectAsState()
            UploadsScreen(
                onNavigateUp = { navController.navigateUp() },
                onNavigateWigleQrScan = { navController.navigate(SettingsRoutes.WigleQrScan) },
                qrImported = qrImported,
                onQrImportConsumed = { entry.savedStateHandle["wigle_qr_imported"] = false },
            )
        }
        composable(SettingsRoutes.WigleQrScan) {
            WigleQrScanScreen(
                onNavigateUp = { navController.navigateUp() },
                onSuccess = {
                    navController.previousBackStackEntry
                        ?.savedStateHandle
                        ?.set("wigle_qr_imported", true)
                    navController.navigateUp()
                },
            )
        }
        composable(SettingsRoutes.Filters) {
            WigleFiltersScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.Detections) {
            DetectionsScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.HomeZone) {
            HomeZoneScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.Location) {
            LocationSourceScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.Notifications) {
            NotificationsScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.Network) {
            NetworkScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.Capture) {
            CaptureScreen()
        }
        composable(SettingsRoutes.Gateway) {
            GatewayScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.ChannelPlan) {
            ChannelPlanScreen(onNavigateUp = { navController.navigateUp() })
        }
        composable(SettingsRoutes.Map) {
            MapSettingsScreen(onNavigateUp = { navController.navigateUp() })
        }
    }
}
