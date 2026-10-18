package net.wardriver.mobile.ui.settings.uploads

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.wardriver.mobile.data.ObservationDataRepository
import net.wardriver.mobile.ui.components.PackAlertDialog
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackOutlinedTextField
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.theme.PackColor
import net.wardriver.mobile.service.WardriveSessionService
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveLocalPref
import androidx.work.WorkInfo
import net.wardriver.mobile.upload.PendingUploadCounts
import net.wardriver.mobile.upload.UploadScheduler
import net.wardriver.mobile.upload.formatUploadStatusLine
import net.wardriver.mobile.upload.loadPendingUploadCounts
import net.wardriver.mobile.upload.rememberUploadUiState
import net.wardriver.mobile.wardriverApp
import okhttp3.OkHttpClient
import okhttp3.Request
import java.util.concurrent.TimeUnit

/** WiGLE + wdgwars upload credentials, manual upload trigger, and local data reset. */
@Composable
fun UploadsScreen(
    onNavigateUp: () -> Unit,
    onNavigateWigleQrScan: () -> Unit = {},
    qrImported: Boolean = false,
    onQrImportConsumed: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val app = context.wardriverApp()
    val appPrefs = app.appPreferences
    val dataRepo = remember { ObservationDataRepository(app.database) }
    val scope = rememberCoroutineScope()

    val savedWigleName by appPrefs.wigleApiNameFlow.collectAsState(initial = "")
    val savedWigleToken by appPrefs.wigleApiTokenFlow.collectAsState(initial = "")
    val savedWdgKey by appPrefs.wdgwarsApiKeyFlow.collectAsState(initial = "")

    var nameDraft by remember(savedWigleName) { mutableStateOf(savedWigleName) }
    var tokenDraft by remember(savedWigleToken) { mutableStateOf(savedWigleToken) }
    var keyDraft by remember(savedWdgKey) { mutableStateOf(savedWdgKey) }
    var credStatus by remember { mutableStateOf<String?>(null) }

    var wdgTestResult by remember { mutableStateOf<String?>(null) }
    var wdgTesting by remember { mutableStateOf(false) }

    var localCounts by remember { mutableStateOf<ObservationDataRepository.LocalDataCounts?>(null) }
    var resetDialogOpen by remember { mutableStateOf(false) }
    var resetInProgress by remember { mutableStateOf(false) }
    var resetMessage by remember { mutableStateOf<String?>(null) }
    var pendingUploads by remember { mutableStateOf<PendingUploadCounts?>(null) }

    val uploadUiState = rememberUploadUiState(context)
    val sessionId by WardriveSessionService.sessionIdFlow.collectAsState()

    suspend fun refreshPendingUploads() {
        pendingUploads = loadPendingUploadCounts(context)
    }

    LaunchedEffect(Unit) {
        localCounts = dataRepo.loadCounts()
        refreshPendingUploads()
    }

    LaunchedEffect(uploadUiState.wigle.workState, uploadUiState.wdgwars.workState) {
        fun finished(state: WorkInfo.State?): Boolean =
            state == WorkInfo.State.SUCCEEDED ||
                state == WorkInfo.State.FAILED ||
                state == WorkInfo.State.CANCELLED
        if (finished(uploadUiState.wigle.workState) || finished(uploadUiState.wdgwars.workState)) {
            refreshPendingUploads()
        }
    }

    LaunchedEffect(qrImported) {
        if (qrImported) {
            credStatus = "WiGLE account imported from QR"
            onQrImportConsumed()
        }
    }

    SettingsSubScreen(
        title = "Uploads",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text("WiGLE API")
        Text(
            text = "Log in at wigle.net/activate and scan the QR code.",
            modifier = Modifier.padding(top = 4.dp),
            style = MaterialTheme.typography.bodySmall,
        )
        PackOutlinedButton(
            text = "Scan WiGLE QR",
            onClick = onNavigateWigleQrScan,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        PackOutlinedTextField(
            value = nameDraft,
            onValueChange = {
                nameDraft = it
                credStatus = null
            },
            label = { Text("API name") },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
            singleLine = true,
        )
        PackOutlinedTextField(
            value = tokenDraft,
            onValueChange = {
                tokenDraft = it
                credStatus = null
            },
            label = { Text("API token") },
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
            singleLine = true,
        )

        Text("wdgwars", modifier = Modifier.padding(top = 16.dp))
        PackOutlinedTextField(
            value = keyDraft,
            onValueChange = {
                keyDraft = it
                credStatus = null
                wdgTestResult = null
            },
            label = { Text("API key") },
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
            singleLine = true,
        )

        PackPrimaryButton(
            text = "Save credentials",
            onClick = {
                saveLocalPref(scope) {
                    appPrefs.setWigleApiName(nameDraft)
                    appPrefs.setWigleApiToken(tokenDraft)
                    appPrefs.setWdgwarsApiKey(keyDraft)
                }
                credStatus = "Saved"
            },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        credStatus?.let { msg ->
            Text(text = msg, modifier = Modifier.padding(top = 4.dp))
        }

        PackPrimaryButton(
            text = if (wdgTesting) "Testing…" else "Test wdgwars key",
            onClick = {
                wdgTesting = true
                wdgTestResult = null
                scope.launch {
                    wdgTestResult = testWdgwarsKey(keyDraft)
                    wdgTesting = false
                }
            },
            enabled = keyDraft.isNotBlank() && !wdgTesting,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        wdgTestResult?.let { msg ->
            Text(text = msg, modifier = Modifier.padding(top = 4.dp))
        }

        pendingUploads?.let { pending ->
            Text(
                text =
                    "${pending.wiglePending} pending WiGLE" +
                        if (keyDraft.isNotBlank()) {
                            ", ${pending.wdgPending} pending wdgwars"
                        } else {
                            ""
                        },
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(top = 16.dp),
            )
        }

        PackPrimaryButton(
            text = if (uploadUiState.anyRunning) "Uploading…" else "Run upload now",
            onClick = {
                UploadScheduler.scheduleAll(context)
                scope.launch { refreshPendingUploads() }
            },
            enabled = !uploadUiState.anyRunning,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        formatUploadStatusLine(uploadUiState.wigle, "WiGLE")?.let { msg ->
            Text(text = msg, modifier = Modifier.padding(top = 4.dp))
        }
        if (keyDraft.isNotBlank()) {
            formatUploadStatusLine(uploadUiState.wdgwars, "wdgwars")?.let { msg ->
                Text(text = msg, modifier = Modifier.padding(top = 4.dp))
            }
        }

        Text("Local data", modifier = Modifier.padding(top = 24.dp))
        localCounts?.let { counts ->
            Text(
                text =
                    "${counts.observations} observations, " +
                        "${counts.sessions} sessions, " +
                        "${counts.detections} detections",
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        PackOutlinedButton(
            text = "Delete all recorded data",
            onClick = { resetDialogOpen = true },
            enabled = !resetInProgress,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        resetMessage?.let { msg ->
            Text(text = msg, modifier = Modifier.padding(top = 4.dp))
        }
    }

    if (resetDialogOpen) {
        PackAlertDialog(
            onDismissRequest = { if (!resetInProgress) resetDialogOpen = false },
            title = "Delete all recorded data?",
            text = {
                val wardriveNote =
                    if (sessionId != null) {
                        "\n\nActive wardrive will be stopped."
                    } else {
                        ""
                    }
                Text(
                    "Removes observations, routes, sessions, detections, and " +
                        "network summaries from this phone. Fleet roster is not deleted.$wardriveNote",
                    color = PackColor.Text,
                )
            },
            confirmText = if (resetInProgress) "Deleting…" else "Delete",
            onConfirm = {
                resetInProgress = true
                resetMessage = null
                scope.launch {
                    when (val result = dataRepo.clearAllObservationData(context)) {
                        is ObservationDataRepository.ResetResult.Ok -> {
                            localCounts = dataRepo.loadCounts()
                            resetMessage =
                                if (result.wardriveWasStopped) {
                                    "Recorded data deleted. Wardrive stopped."
                                } else {
                                    "Recorded data deleted."
                                }
                            resetDialogOpen = false
                        }
                        ObservationDataRepository.ResetResult.WardriveStopTimeout -> {
                            resetMessage = "Could not stop wardrive - try again."
                        }
                    }
                    resetInProgress = false
                }
            },
            confirmEnabled = !resetInProgress,
            dismissEnabled = !resetInProgress,
            destructive = true,
        )
    }
}

/** GET https://wdgwars.pl/api/me to validate API key. */
private suspend fun testWdgwarsKey(apiKey: String): String =
    withContext(Dispatchers.IO) {
        if (apiKey.isBlank()) return@withContext "Enter an API key first"
        val client =
            OkHttpClient.Builder()
                .connectTimeout(15, TimeUnit.SECONDS)
                .readTimeout(15, TimeUnit.SECONDS)
                .build()
        val request =
            Request.Builder()
                .url("https://wdgwars.pl/api/me")
                .header("X-API-Key", apiKey)
                .get()
                .build()
        try {
            client.newCall(request).execute().use { resp ->
                val body = resp.body?.string().orEmpty()
                if (resp.isSuccessful) "OK: $body" else "HTTP ${resp.code}: $body"
            }
        } catch (e: Exception) {
            "Error: ${e.message}"
        }
    }
