package net.wardriver.mobile

import android.app.Application
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.data.FleetRepository
import net.wardriver.mobile.data.GatewayWifiRepository
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.db.WardriverDatabase

/** Application singleton: gateway SoftAP link, Room DB, fleet roster. */
class WardriverApp : Application() {
    lateinit var gatewayWifiRepository: GatewayWifiRepository
        private set

    lateinit var appPreferences: AppPreferences
        private set

    lateinit var fleetRepository: FleetRepository
        private set

    val database: WardriverDatabase by lazy { WardriverDatabase.getInstance(this) }

    override fun onCreate() {
        super.onCreate()
        appPreferences = AppPreferences(this)
        gatewayWifiRepository = GatewayWifiRepository(this, appPreferences)
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            appPreferences.migrateLegacyUploadPrefsIfNeeded()
            appPreferences.migrateLegacyDetectionKeysIfNeeded()
            appPreferences.purgeLegacyGeofenceDetectionsIfNeeded(database)
            reconcileOrphanSessions()
        }
        fleetRepository = FleetRepository(this, database.fleetRosterDao())
    }

    private suspend fun reconcileOrphanSessions() {
        val prefs = appPreferences
        val activeId = if (prefs.wardriveActive()) prefs.activeWardriveSessionId() else null
        val now = System.currentTimeMillis()
        if (activeId != null) {
            database.sessionDao().closeOrphansExcept(activeId, now)
            val session = database.sessionDao().get(activeId)
            if (session != null && session.endedUnixMs == null) {
                WardriveSessionService.restoreSessionId(activeId)
            } else {
                prefs.setWardriveActive(active = false)
            }
        } else {
            for (orphan in database.sessionDao().getOpenSessions()) {
                database.sessionDao().update(orphan.copy(endedUnixMs = now))
            }
        }
    }
}

/** Typed access to [WardriverApp] from any [android.content.Context]. */
fun android.content.Context.wardriverApp(): WardriverApp =
    applicationContext as WardriverApp
