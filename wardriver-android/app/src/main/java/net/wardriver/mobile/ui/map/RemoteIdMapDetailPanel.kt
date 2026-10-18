package net.wardriver.mobile.ui.map

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor
import kotlin.math.roundToInt

/** Fraction of screen height for the default RID detail panel. */
const val REMOTE_ID_PANEL_HEIGHT_FRACTION = 0.40f

/** Max fraction of screen height when the user drags the panel taller. */
const val REMOTE_ID_PANEL_MAX_HEIGHT_FRACTION = 0.60f

/**
 * Non-modal bottom panel for Remote ID alert detail.
 * Map above the panel stays pannable; body scrolls independently.
 */
@Composable
fun RemoteIdMapDetailPanel(
    pin: AlertPin,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val config = LocalConfiguration.current
    val screenHeightDp = config.screenHeightDp.toFloat()
    val density = LocalDensity.current
    var heightFraction by remember { mutableFloatStateOf(REMOTE_ID_PANEL_HEIGHT_FRACTION) }
    val panelHeightDp = (screenHeightDp * heightFraction).roundToInt().dp

    Surface(
        modifier =
            modifier
                .fillMaxWidth()
                .height(panelHeightDp),
        shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
        color = PackColor.Surface2,
        tonalElevation = 6.dp,
        shadowElevation = 8.dp,
    ) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .pointerInput(Unit) {
                        detectVerticalDragGestures { change, dragAmount ->
                            change.consume()
                            val deltaFraction = -with(density) { dragAmount.toDp() }.value / screenHeightDp
                            heightFraction =
                                (heightFraction + deltaFraction)
                                    .coerceIn(REMOTE_ID_PANEL_HEIGHT_FRACTION, REMOTE_ID_PANEL_MAX_HEIGHT_FRACTION)
                        }
                    }
                    .padding(top = 8.dp, bottom = 4.dp),
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    Modifier
                        .width(36.dp)
                        .height(4.dp)
                        .background(
                            PackColor.Muted.copy(alpha = 0.5f),
                            RoundedCornerShape(2.dp),
                        ),
                )
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = pin.title,
                    style = MaterialTheme.typography.titleMedium,
                    modifier =
                        Modifier
                            .weight(1f)
                            .padding(start = 8.dp),
                )
                IconButton(onClick = onDismiss) {
                    Icon(
                        Icons.Filled.Close,
                        contentDescription = "Close Remote ID detail",
                        tint = PackColor.Muted,
                    )
                }
            }
            RemoteIdDetailRows(
                pin = pin,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp)
                        .padding(bottom = 16.dp),
            )
        }
    }
}

/** Bottom padding in px for map zoom-to-fit when the RID panel is open at default height. */
fun remoteIdPanelBottomPaddingPx(screenHeightPx: Int): Int =
    (screenHeightPx * REMOTE_ID_PANEL_HEIGHT_FRACTION).roundToInt()
