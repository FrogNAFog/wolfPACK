package net.wardriver.mobile.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb

/** PACK / wolfPACK design tokens — mirrors `wardriver-control/assets/web_theme.css`. */
object PackColor {
    val Background = Color(0xFF0A0612)
    val Header = Color(0xFF12081F)
    val Surface = Color(0xFF161024)
    val Surface2 = Color(0xFF1C1530)
    val Text = Color(0xFFD8D4E8)
    val Muted = Color(0xFF8B849E)
    val Cyan = Color(0xFF2EE8F5)
    val Chrome = Color(0xFFFF2D92)
    val Sun = Color(0xFFFFB347)
    val Good = Color(0xFF5EE0A8)
    val Warn = Color(0xFFFBBF24)
    val Err = Color(0xFFFF7A8A)

    val Ink = Color(0xF00A0612)
    val BorderCyan = Color(0x242EE8F5)
    val BorderChrome = Color(0x38FF2D92)
    val BorderChromeSubtle = Color(0x33FF2D92)
    val HotWash = Color(0x29FFB347)
    val HotWashSoft = Color(0x1AFFB347)
    val ErrWash = Color(0x29FF7A8A)
    val CoolWash = Color(0x1F2EE8F5)
    val NavElevated = Color(0xF00A0612)
    val WarnBorder = Color(0xE0FBBF24)

    val DetectionBannerTop = Color(0xF72D2008)
    val DetectionBannerBottom = Color(0xF5030B18)

    val MapWifi = Color(0xFF2C7BE5)
    val MapBle = Color(0xFFE67E22)
    val MapProbe = Color(0xFF9B59B6)
    val MapAlert = Err
    val MapGeofenceFill = Color(0x331A7F37)
    val MapGeofencePreviewFill = Color(0x33FFB347)
    val MapCoverageFill = Color(0xCCE74C3C)

    fun argb(color: Color): Int = color.toArgb()
}
