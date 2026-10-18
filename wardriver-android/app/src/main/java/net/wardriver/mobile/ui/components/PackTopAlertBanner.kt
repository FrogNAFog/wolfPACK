package net.wardriver.mobile.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import net.wardriver.mobile.ui.theme.PackColor

private const val AUTO_DISMISS_MS = 6_000L

@Composable
fun PackTopAlertBanner(
    title: String?,
    subtitle: String?,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    titleColor: Color = PackColor.Sun,
    subtitleFontFamily: FontFamily = FontFamily.Default,
) {
    val context = LocalContext.current
    val reduceMotion =
        android.provider.Settings.Global.getFloat(
            context.contentResolver,
            android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,
            1f,
        ) == 0f

    LaunchedEffect(title, subtitle) {
        if (title != null) {
            delay(AUTO_DISMISS_MS)
            onDismiss()
        }
    }

    val animMillis = if (reduceMotion) 0 else 200
    AnimatedVisibility(
        visible = title != null && subtitle != null,
        enter =
            fadeIn(tween(animMillis)) +
                slideInVertically(tween(animMillis)) { -it },
        exit =
            fadeOut(tween(animMillis)) +
                slideOutVertically(tween(animMillis)) { -it },
        modifier = modifier,
    ) {
        if (title == null || subtitle == null) return@AnimatedVisibility
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(bottomStart = 10.dp, bottomEnd = 10.dp))
                    .background(
                        Brush.verticalGradient(
                            colors = listOf(PackColor.DetectionBannerTop, PackColor.DetectionBannerBottom),
                        ),
                    )
                    .border(
                        width = 2.dp,
                        color = PackColor.WarnBorder,
                        shape = RoundedCornerShape(bottomStart = 10.dp, bottomEnd = 10.dp),
                    )
                    .clickable(onClick = onDismiss)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                color = titleColor,
            )
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Muted,
                fontFamily = subtitleFontFamily,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}
