package net.wardriver.mobile.ui.map

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import net.wardriver.mobile.data.ApMarkerKind
import net.wardriver.mobile.ui.theme.PackColor

/** Builds and caches osmdroid marker icons with PACK WiFi / BLE / alert colors. */
object MapMarkerFactory {
    private const val PIN_SIZE_DP = 28
    private const val STROKE_DP = 2f

    private val apIconCache = mutableMapOf<String, Drawable>()
    private val alertIconCache = mutableMapOf<String, Drawable>()
    private var operatorIconCache: Drawable? = null

    fun apIcon(
        context: Context,
        signalKind: String,
    ): Drawable {
        val key = signalKind.uppercase()
        return apIconCache.getOrPut(key) {
            val color =
                when (key) {
                    "BLE" -> PackColor.MapBle
                    ApMarkerKind.PROBE_REQ -> PackColor.MapProbe
                    else -> PackColor.MapWifi
                }
            circlePin(context, PackColor.argb(color))
        }
    }

    fun alertIcon(
        context: Context,
        kind: String,
    ): Drawable {
        val key = kind.uppercase()
        return alertIconCache.getOrPut(key) {
            circlePin(context, PackColor.argb(PackColor.MapAlert))
        }
    }

    fun operatorIcon(context: Context): Drawable =
        operatorIconCache ?: circlePin(context, PackColor.argb(PackColor.Sun)).also {
            operatorIconCache = it
        }

    private fun circlePin(
        context: Context,
        fillArgb: Int,
    ): Drawable {
        val density = context.resources.displayMetrics.density
        val sizePx = (PIN_SIZE_DP * density).toInt().coerceAtLeast(16)
        val strokePx = STROKE_DP * density
        val bitmap = Bitmap.createBitmap(sizePx, sizePx, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val radius = sizePx / 2f - strokePx
        val cx = sizePx / 2f
        val cy = sizePx / 2f
        val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = fillArgb }
        val stroke =
            Paint(Paint.ANTI_ALIAS_FLAG).apply {
                style = Paint.Style.STROKE
                strokeWidth = strokePx
                color = PackColor.argb(PackColor.Text)
            }
        canvas.drawCircle(cx, cy, radius, fill)
        canvas.drawCircle(cx, cy, radius, stroke)
        return BitmapDrawable(context.resources, bitmap).apply {
            setBounds(0, 0, sizePx, sizePx)
        }
    }

    /** Test hook: resolve AP pin color ARGB without creating a drawable. */
    internal fun apFillArgb(signalKind: String): Int {
        val key = signalKind.uppercase()
        val color =
            when (key) {
                "BLE" -> PackColor.MapBle
                else -> PackColor.MapWifi
            }
        return PackColor.argb(color)
    }

    internal fun alertFillArgb(): Int = PackColor.argb(PackColor.MapAlert)
}
