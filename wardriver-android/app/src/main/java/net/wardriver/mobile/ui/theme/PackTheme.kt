package net.wardriver.mobile.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val PackColorScheme =
    darkColorScheme(
        primary = PackColor.Cyan,
        onPrimary = PackColor.Background,
        secondary = PackColor.Chrome,
        onSecondary = PackColor.Text,
        tertiary = PackColor.Sun,
        onTertiary = PackColor.Background,
        background = PackColor.Background,
        onBackground = PackColor.Text,
        surface = PackColor.Surface,
        onSurface = PackColor.Text,
        surfaceVariant = PackColor.Surface2,
        onSurfaceVariant = PackColor.Muted,
        surfaceContainerLow = PackColor.Surface2,
        surfaceContainer = PackColor.Surface,
        surfaceContainerHigh = PackColor.Header,
        error = PackColor.Err,
        onError = PackColor.Background,
        errorContainer = PackColor.ErrWash,
        onErrorContainer = PackColor.Err,
        outline = PackColor.BorderCyan,
        outlineVariant = PackColor.BorderChromeSubtle,
        inverseSurface = PackColor.Surface2,
        inverseOnSurface = PackColor.Text,
        scrim = Color(0xCC0A0612),
    )

@Composable
fun PackTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = PackColorScheme,
        typography = PackTypography,
        shapes = PackShapes,
        content = content,
    )
}
