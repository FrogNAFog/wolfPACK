package net.wardriver.mobile.ui.components

import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.VisualTransformation
import net.wardriver.mobile.ui.theme.PackColor

@Composable
fun PackOutlinedTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    label: @Composable (() -> Unit)? = null,
    placeholder: @Composable (() -> Unit)? = null,
    supportingText: @Composable (() -> Unit)? = null,
    isError: Boolean = false,
    singleLine: Boolean = false,
    minLines: Int = 1,
    readOnly: Boolean = false,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier,
        label = label,
        placeholder = placeholder,
        supportingText = supportingText,
        isError = isError,
        singleLine = singleLine,
        minLines = minLines,
        readOnly = readOnly,
        visualTransformation = visualTransformation,
        keyboardOptions = keyboardOptions,
        colors =
            OutlinedTextFieldDefaults.colors(
                focusedTextColor = PackColor.Text,
                unfocusedTextColor = PackColor.Text,
                disabledTextColor = PackColor.Muted,
                focusedContainerColor = PackColor.Ink,
                unfocusedContainerColor = PackColor.Ink,
                disabledContainerColor = PackColor.Surface2,
                focusedBorderColor = PackColor.Cyan,
                unfocusedBorderColor = PackColor.BorderChrome,
                errorBorderColor = PackColor.Err,
                focusedLabelColor = PackColor.Cyan,
                unfocusedLabelColor = PackColor.Muted,
                cursorColor = PackColor.Cyan,
            ),
    )
}

@Composable
fun PackSwitch(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Switch(
        checked = checked,
        onCheckedChange = onCheckedChange,
        modifier = modifier,
        enabled = enabled,
        colors =
            SwitchDefaults.colors(
                checkedThumbColor = PackColor.Background,
                checkedTrackColor = PackColor.Cyan,
                uncheckedThumbColor = PackColor.Muted,
                uncheckedTrackColor = PackColor.Surface2,
                disabledCheckedThumbColor = PackColor.Muted,
                disabledCheckedTrackColor = PackColor.Surface,
                disabledUncheckedThumbColor = PackColor.Muted,
                disabledUncheckedTrackColor = PackColor.Surface,
            ),
    )
}

@Composable
fun PackRadioButton(
    selected: Boolean,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    RadioButton(
        selected = selected,
        onClick = onClick,
        modifier = modifier,
        enabled = enabled,
        colors =
            RadioButtonDefaults.colors(
                selectedColor = PackColor.Cyan,
                unselectedColor = PackColor.Muted,
                disabledSelectedColor = PackColor.Muted,
                disabledUnselectedColor = PackColor.Muted.copy(alpha = 0.38f),
            ),
    )
}
