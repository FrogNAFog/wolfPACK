package net.wardriver.mobile.ui.components

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import net.wardriver.mobile.ui.components.PackOutlinedTextField
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.theme.PackColor

/**
 * Multiline text area with local draft state and explicit save.
 * Avoids cursor jump from DataStore Flow round-trips on every keystroke.
 */
@Composable
fun SavedMultilineField(
    savedValue: String,
    label: String,
    saveButtonText: String,
    onSave: suspend (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String? = null,
    minLines: Int = 4,
    validate: (String) -> String? = { null },
    normalize: (String) -> String = { it },
) {
    val scope = rememberCoroutineScope()
    var draft by remember(savedValue) { mutableStateOf(savedValue) }
    var error by remember { mutableStateOf<String?>(null) }
    var status by remember { mutableStateOf<String?>(null) }

    PackOutlinedTextField(
        value = draft,
        onValueChange = {
            draft = it
            error = null
            status = null
        },
        label = { Text(label) },
        placeholder = placeholder?.let { { Text(it) } },
        isError = error != null,
        supportingText = error?.let { err -> { Text(err, color = PackColor.Err) } },
        modifier = modifier.fillMaxWidth(),
        minLines = minLines,
    )
    PackPrimaryButton(
        text = saveButtonText,
        onClick = {
            val validationError = validate(draft)
            if (validationError != null) {
                error = validationError
                status = null
                return@PackPrimaryButton
            }
            val normalized = normalize(draft)
            draft = normalized
            scope.launch {
                onSave(normalized)
                error = null
                status = "Saved"
            }
        },
        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
    )
    status?.let { saved ->
        Text(
            text = saved,
            color = PackColor.Muted,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}
