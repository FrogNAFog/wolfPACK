package net.wardriver.mobile.ui.nav

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import net.wardriver.mobile.notify.AlertBus
import net.wardriver.mobile.ui.RequestWardriverPermissions
import net.wardriver.mobile.ui.components.PackDetectionAlertBanner
import net.wardriver.mobile.ui.components.PackScreenBackground
import net.wardriver.mobile.ui.components.PackTopAlertBanner
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.ui.theme.PackTheme

@Composable
fun MainScaffold(modifier: Modifier = Modifier) {
    RequestWardriverPermissions()

    val navController = rememberNavController()
    val backStack by navController.currentBackStackEntryAsState()
    val currentRoute = backStack?.destination?.route
    var detectionAlert by remember { mutableStateOf<AlertBus.Message?>(null) }
    var geofenceAlert by remember { mutableStateOf<AlertBus.Message?>(null) }

    androidx.compose.runtime.LaunchedEffect(Unit) {
        AlertBus.events.collect { message ->
            when (message.kind) {
                AlertBus.Kind.Detection -> detectionAlert = message
                AlertBus.Kind.GeofenceEnter,
                AlertBus.Kind.GeofenceLeave,
                -> geofenceAlert = message
            }
        }
    }

    PackTheme {
        PackScreenBackground {
            Scaffold(
                modifier = modifier,
                containerColor = androidx.compose.ui.graphics.Color.Transparent,
                bottomBar = {
                    NavigationBar(
                        containerColor = PackColor.NavElevated,
                        tonalElevation = 0.dp,
                    ) {
                        AppDestinations.entries.forEach { destination ->
                            val selected = currentRoute == destination.route
                            NavigationBarItem(
                                selected = selected,
                                onClick = {
                                    navController.navigate(destination.route) {
                                        popUpTo(navController.graph.startDestinationId) {
                                            saveState = true
                                        }
                                        launchSingleTop = true
                                        restoreState = true
                                    }
                                },
                                icon = {
                                    Icon(
                                        imageVector = destination.icon,
                                        contentDescription = destination.label,
                                    )
                                },
                                label = { Text(destination.label) },
                                colors =
                                    NavigationBarItemDefaults.colors(
                                        selectedIconColor = PackColor.Background,
                                        selectedTextColor = PackColor.Background,
                                        unselectedIconColor = PackColor.Muted,
                                        unselectedTextColor = PackColor.Muted,
                                        indicatorColor = PackColor.Cyan,
                                    ),
                            )
                        }
                    }
                },
            ) { innerPadding ->
                Box(Modifier.fillMaxSize().padding(innerPadding)) {
                    AppNavHost(
                        navController = navController,
                        modifier = Modifier.fillMaxSize(),
                    )
                    val geo = geofenceAlert
                    if (geo != null) {
                        PackTopAlertBanner(
                            title = geo.title,
                            subtitle = geo.detail,
                            onDismiss = { geofenceAlert = null },
                            modifier =
                                Modifier
                                    .align(Alignment.TopCenter)
                                    .statusBarsPadding(),
                            titleColor =
                                if (geo.kind == AlertBus.Kind.GeofenceEnter) {
                                    PackColor.Good
                                } else {
                                    PackColor.Sun
                                },
                        )
                    } else {
                        PackDetectionAlertBanner(
                            message = detectionAlert,
                            onDismiss = { detectionAlert = null },
                            modifier =
                                Modifier
                                    .align(Alignment.TopCenter)
                                    .statusBarsPadding(),
                        )
                    }
                }
            }
        }
    }
}
