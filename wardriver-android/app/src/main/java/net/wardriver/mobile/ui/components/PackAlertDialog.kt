package net.wardriver.mobile.ui.components

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun PackAlertDialog(
    onDismissRequest: () -> Unit,
    title: String,
    text: @Composable () -> Unit,
    confirmText: String,
    onConfirm: () -> Unit,
    modifier: Modifier = Modifier,
    dismissText: String = "Cancel",
    onDismiss: (() -> Unit)? = null,
    confirmEnabled: Boolean = true,
    dismissEnabled: Boolean = true,
    destructive: Boolean = false,
) {
    AlertDialog(
        onDismissRequest = onDismissRequest,
        modifier = modifier,
        containerColor = PackColor.Surface,
        titleContentColor = PackColor.Cyan,
        textContentColor = PackColor.Text,
        title = {
            Text(text = title, style = MaterialTheme.typography.titleLarge, color = PackColor.Cyan)
        },
        text = text,
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = confirmEnabled) {
                Text(
                    text = confirmText,
                    color = if (destructive) PackColor.Err else PackColor.Cyan,
                )
            }
        },
        dismissButton = {
            TextButton(onClick = { onDismiss?.invoke() ?: onDismissRequest() }, enabled = dismissEnabled) {
                Text(text = dismissText, color = PackColor.Muted)
            }
        },
    )
}
