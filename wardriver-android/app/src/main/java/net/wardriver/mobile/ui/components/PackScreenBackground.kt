package net.wardriver.mobile.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import net.wardriver.mobile.ui.theme.PackColor

/** Static ambient background matching PACK `body::before` gradients (no animation). */
@Composable
fun PackScreenBackground(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Box(
        modifier =
            modifier
                .fillMaxSize()
                .background(PackColor.Background)
                .background(
                    Brush.verticalGradient(
                        colors = listOf(Color(0xFF140A22), PackColor.Background),
                        startY = 0f,
                        endY = 1200f,
                    ),
                )
                .background(
                    Brush.radialGradient(
                        colors = listOf(Color(0x24FF2D92), Color.Transparent),
                        radius = 900f,
                    ),
                )
                .background(
                    Brush.radialGradient(
                        colors = listOf(Color(0x142EE8F5), Color.Transparent),
                        radius = 700f,
                    ),
                ),
    ) {
        content()
    }
}

/** Uppercase chrome/cyan section label (`.k` / glance label). */
@Composable
fun PackSectionLabel(
    text: String,
    modifier: Modifier = Modifier,
) {
    androidx.compose.material3.Text(
        text = text.uppercase(),
        style = androidx.compose.material3.MaterialTheme.typography.labelMedium,
        color = PackColor.Cyan,
        modifier = modifier,
    )
}
