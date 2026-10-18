package net.wardriver.mobile.data

/** How detection alerts surface to the user on the phone. */
enum class AlertDeliveryMode {
    /** Snackbar while app is foreground; system notification when background. */
    InAppWhenForeground,
    /** Heads-up system notification with sound/vibration toggles. */
    Full,
    /** Shade-only system notification (no heads-up). */
    Quiet,
}
