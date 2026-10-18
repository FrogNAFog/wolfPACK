package net.wardriver.mobile.upload

/** Parsed WiGLE activate QR credentials (`username:authname:token`). */
data class WigleQrCredentials(
    val apiName: String,
    val apiToken: String,
)

private val WIGLE_ACTIVATE_QR_REGEX = Regex("""^.*:[a-zA-Z0-9]*:[a-zA-Z0-9]*$""")

/**
 * Parse a QR payload from https://wigle.net/activate (same format as WiGLE Android).
 * Returns API name + token; login username (first field) is not used for upload auth.
 */
fun parseWigleActivateQr(raw: String): WigleQrCredentials? {
    val trimmed = raw.trim()
    if (!WIGLE_ACTIVATE_QR_REGEX.matches(trimmed)) return null
    val parts = trimmed.split(":")
    if (parts.size != 3) return null
    val apiName = parts[1]
    val apiToken = parts[2]
    if (apiName.isBlank() || apiToken.isBlank()) return null
    return WigleQrCredentials(apiName = apiName, apiToken = apiToken)
}
