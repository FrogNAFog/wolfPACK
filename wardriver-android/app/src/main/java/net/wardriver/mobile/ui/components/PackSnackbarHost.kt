package net.wardriver.mobile.ui.components

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarData
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun PackSnackbarHost(
    hostState: SnackbarHostState,
    modifier: Modifier = Modifier,
) {
    SnackbarHost(
        hostState = hostState,
        modifier = modifier,
    ) { data ->
        PackSnackbar(data)
    }
}

@Composable
private fun PackSnackbar(data: SnackbarData) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
        contentAlignment = Alignment.BottomCenter,
    ) {
        Surface(
            shape = RoundedCornerShape(10.dp),
            color = PackColor.Surface2,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .border(1.dp, PackColor.BorderCyan, RoundedCornerShape(10.dp)),
        ) {
            Text(
                text = data.visuals.message,
                style = MaterialTheme.typography.bodyMedium,
                color = PackColor.Text,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }
    }
}
