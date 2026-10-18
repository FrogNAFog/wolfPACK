package net.wardriver.mobile.data

/** Session location source for the wardrive foreground service. */
enum class LocationSource {
    /**
     * Platform [android.location.LocationManager] GPS provider. Fixes stay on-device with
     * zero Google-server involvement. This is the default.
     */
    GPS_PROVIDER_ONLY,

    /**
     * Google fused provider (opt-in). May use WiFi and cell network positioning that queries
     * Google's location service, giving faster fixes indoors and before the first GPS lock.
     */
    FUSED_OPT_IN,
}
