package net.wardriver.mobile.wifi

import android.content.Context
import android.content.SharedPreferences
import java.util.UUID

/** Phone-local TOFU state: stable phone id for gateway pinning. */
class GatewayTrustStore(context: Context) {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun phoneId(): String {
        val existing = prefs.getString(KEY_PHONE_ID, null)
        if (!existing.isNullOrBlank()) return existing
        val id = UUID.randomUUID().toString()
        prefs.edit().putString(KEY_PHONE_ID, id).apply()
        return id
    }

    fun hasTrustedGateway(): Boolean = prefs.getBoolean(KEY_TRUSTED, false)

    fun markTrusted() {
        prefs.edit().putBoolean(KEY_TRUSTED, true).apply()
    }

    fun forgetGateway() {
        prefs.edit()
            .remove(KEY_TRUSTED)
            .apply()
    }

    companion object {
        private const val PREFS = "gateway_trust"
        private const val KEY_PHONE_ID = "phone_id"
        private const val KEY_TRUSTED = "trusted"
    }
}
