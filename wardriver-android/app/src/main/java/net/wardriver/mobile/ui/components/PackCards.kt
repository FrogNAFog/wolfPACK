package net.wardriver.mobile.ui.components

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

/** Dashboard glance tile — `.dashboard-glance__card`. */
@Composable
fun PackGlanceCard(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    foot: String? = null,
    valueColor: Color = PackColor.Text,
    footColor: Color = PackColor.Text,
    valueAnnotated: AnnotatedString? = null,
) {
    Surface(
        modifier =
            modifier
                .border(1.dp, PackColor.BorderChromeSubtle, RoundedCornerShape(10.dp)),
        shape = RoundedCornerShape(10.dp),
        color = PackColor.Surface2,
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text(
                text = label.uppercase(),
                style = MaterialTheme.typography.labelSmall,
                color = PackColor.Cyan,
            )
            if (valueAnnotated != null) {
                Text(
                    text = valueAnnotated,
                    style = MaterialTheme.typography.headlineSmall.copy(fontFamily = FontFamily.Monospace),
                    modifier = Modifier.padding(top = 4.dp),
                )
            } else {
                Text(
                    text = value,
                    style = MaterialTheme.typography.headlineSmall.copy(fontFamily = FontFamily.Monospace),
                    color = valueColor,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
            foot?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodySmall,
                    color = footColor,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
        }
    }
}

/** General content card — `.card`. */
@Composable
fun PackCard(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier =
            modifier
                .fillMaxWidth()
                .border(1.dp, PackColor.BorderCyan, RoundedCornerShape(10.dp)),
        shape = RoundedCornerShape(10.dp),
        color = PackColor.Surface,
        content = {
            Column(modifier = Modifier.padding(12.dp), content = content)
        },
    )
}

/** Detection alert banner. */
@Composable
fun PackAlertBanner(
    headline: String,
    subtitle: String,
    modifier: Modifier = Modifier,
    isError: Boolean = true,
) {
    val border = if (isError) PackColor.Err else PackColor.Sun
    val bg = if (isError) PackColor.ErrWash else PackColor.HotWash
    Surface(
        modifier =
            modifier
                .fillMaxWidth()
                .border(1.dp, border, RoundedCornerShape(10.dp)),
        shape = RoundedCornerShape(10.dp),
        color = bg,
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text(
                text = headline,
                style = MaterialTheme.typography.titleSmall,
                color = if (isError) PackColor.Err else PackColor.Sun,
            )
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Text,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}
