package net.wardriver.mobile.ui.map

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun MapAttribution(
    basemap: MapBasemap,
    modifier: Modifier = Modifier,
) {
    Text(
        text = PackTileSources.attribution(basemap),
        style = MaterialTheme.typography.labelSmall,
        color = PackColor.Muted,
        modifier =
            modifier
                .background(PackColor.Surface, RoundedCornerShape(6.dp))
                .padding(horizontal = 8.dp, vertical = 4.dp),
    )
}
