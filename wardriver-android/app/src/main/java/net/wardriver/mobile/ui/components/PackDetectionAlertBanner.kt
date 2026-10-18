package net.wardriver.mobile.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import net.wardriver.mobile.notify.AlertBus
import net.wardriver.mobile.notify.AlertThrottle
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun PackDetectionAlertBanner(
    message: AlertBus.Message?,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val mac = message?.detail?.let { AlertThrottle.parseMac(it) }
    val subtitle =
        message?.let {
            buildString {
                if (mac != null) {
                    append(mac)
                    val rest = it.detail.replace(Regex("""mac=$mac""", RegexOption.IGNORE_CASE), "").trim()
                    if (rest.isNotBlank()) append(" · ").append(rest)
                } else {
                    append(it.detail)
                }
                if (it.suppressedCount > 0) {
                    append(" (+${it.suppressedCount})")
                }
            }
        }
    PackTopAlertBanner(
        title = message?.title,
        subtitle = subtitle,
        onDismiss = onDismiss,
        modifier = modifier,
        titleColor = PackColor.Sun,
        subtitleFontFamily = if (mac != null) FontFamily.Monospace else FontFamily.Default,
    )
}
