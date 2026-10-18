package net.wardriver.mobile.ui.components

import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun PackPrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val highlighted = enabled && pressed

    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier,
        shape = RoundedCornerShape(8.dp),
        interactionSource = interactionSource,
        colors =
            ButtonDefaults.buttonColors(
                containerColor = if (highlighted) PackColor.Cyan.copy(alpha = 0.85f) else PackColor.Cyan,
                contentColor = PackColor.Background,
                disabledContainerColor = PackColor.Surface2,
                disabledContentColor = PackColor.Muted,
            ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Text(text, color = PackColor.Background)
    }
}

@Composable
fun PackOutlinedButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val highlighted = enabled && pressed

    OutlinedButton(
        onClick = onClick,
        enabled = enabled,
        interactionSource = interactionSource,
        modifier =
            modifier.border(
                width = 1.dp,
                color = if (enabled) PackColor.BorderChrome else PackColor.BorderChromeSubtle,
                shape = RoundedCornerShape(8.dp),
            ),
        shape = RoundedCornerShape(8.dp),
        colors =
            ButtonDefaults.outlinedButtonColors(
                contentColor = if (highlighted) PackColor.Background else PackColor.Cyan,
                containerColor = if (highlighted) PackColor.Cyan else Color.Transparent,
                disabledContentColor = PackColor.Muted,
                disabledContainerColor = Color.Transparent,
            ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Text(text, color = if (highlighted) PackColor.Background else PackColor.Cyan)
    }
}
