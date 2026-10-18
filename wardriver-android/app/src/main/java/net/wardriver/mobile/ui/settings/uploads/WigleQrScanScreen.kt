package net.wardriver.mobile.ui.settings.uploads

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import net.wardriver.mobile.ui.settings.SettingsSubScreen
import net.wardriver.mobile.ui.settings.saveLocalPref
import net.wardriver.mobile.upload.parseWigleActivateQr
import net.wardriver.mobile.wardriverApp
import java.util.concurrent.atomic.AtomicBoolean

private const val WIGLE_ACTIVATE_URL = "https://wigle.net/activate"

@Composable
fun WigleQrScanScreen(
    onNavigateUp: () -> Unit,
    onSuccess: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()
    val appPrefs = context.wardriverApp().appPreferences

    var hasCameraPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    var statusText by remember { mutableStateOf<String?>(null) }
    val scanHandled = remember { AtomicBoolean(false) }
    val previewView = remember { PreviewView(context) }

    val permissionLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            hasCameraPermission = granted
            if (!granted) {
                statusText = "Camera permission is required to scan the WiGLE QR code."
            }
        }

    LaunchedEffect(Unit) {
        if (!hasCameraPermission) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    DisposableEffect(hasCameraPermission, lifecycleOwner) {
        if (!hasCameraPermission || scanHandled.get()) {
            return@DisposableEffect onDispose {}
        }

        val cameraProviderFuture = ProcessCameraProvider.getInstance(context)
        val executor = ContextCompat.getMainExecutor(context)
        var cameraProvider: ProcessCameraProvider? = null

        cameraProviderFuture.addListener(
            {
                if (scanHandled.get()) return@addListener
                val provider = cameraProviderFuture.get()
                cameraProvider = provider
                provider.unbindAll()

                val preview =
                    Preview.Builder().build().also {
                        it.surfaceProvider = previewView.surfaceProvider
                    }

                val barcodeScanner =
                    BarcodeScanning.getClient(
                        BarcodeScannerOptions.Builder()
                            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                            .build(),
                    )

                val analysis =
                    ImageAnalysis.Builder()
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build()

                analysis.setAnalyzer(executor) { imageProxy ->
                    processQrFrame(
                        imageProxy = imageProxy,
                        barcodeScanner = barcodeScanner,
                        scanHandled = scanHandled,
                        onInvalidQr = { statusText = "Invalid WiGLE QR code" },
                        onValidQr = { creds ->
                            if (!scanHandled.compareAndSet(false, true)) return@processQrFrame
                            saveLocalPref(scope) {
                                appPrefs.setWigleApiName(creds.apiName)
                                appPrefs.setWigleApiToken(creds.apiToken)
                            }
                            onSuccess()
                        },
                    )
                }

                try {
                    provider.bindToLifecycle(
                        lifecycleOwner,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analysis,
                    )
                } catch (_: Exception) {
                    statusText = "Unable to start camera."
                }
            },
            executor,
        )

        onDispose {
            cameraProvider?.unbindAll()
        }
    }

    SettingsSubScreen(
        title = "Scan WiGLE QR",
        onNavigateUp = onNavigateUp,
        showPendingGatewayChip = false,
        modifier = modifier,
    ) {
        Text(
            text = "Log in at wigle.net/activate and scan the QR code shown on that page.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(bottom = 8.dp),
        )
        TextButton(
            onClick = {
                context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(WIGLE_ACTIVATE_URL)))
            },
        ) {
            Text("Open wigle.net/activate")
        }

        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(360.dp)
                    .padding(top = 8.dp),
            contentAlignment = Alignment.Center,
        ) {
            when {
                !hasCameraPermission -> {
                    Text(
                        text = statusText ?: "Requesting camera permission…",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                else -> {
                    AndroidView(
                        factory = { previewView },
                        modifier = Modifier.fillMaxWidth().height(360.dp),
                    )
                }
            }
        }

        statusText?.let { msg ->
            Text(
                text = msg,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(top = 8.dp),
            )
        }
    }
}

@androidx.annotation.OptIn(ExperimentalGetImage::class)
private fun processQrFrame(
    imageProxy: ImageProxy,
    barcodeScanner: BarcodeScanner,
    scanHandled: AtomicBoolean,
    onInvalidQr: () -> Unit,
    onValidQr: (net.wardriver.mobile.upload.WigleQrCredentials) -> Unit,
) {
    if (scanHandled.get()) {
        imageProxy.close()
        return
    }
    val mediaImage = imageProxy.image
    if (mediaImage == null) {
        imageProxy.close()
        return
    }
    val inputImage = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
    barcodeScanner
        .process(inputImage)
        .addOnSuccessListener { barcodes ->
            if (scanHandled.get()) {
                imageProxy.close()
                return@addOnSuccessListener
            }
            for (barcode in barcodes) {
                val raw = barcode.rawValue ?: continue
                val creds = parseWigleActivateQr(raw)
                if (creds != null) {
                    onValidQr(creds)
                    imageProxy.close()
                    return@addOnSuccessListener
                }
            }
            if (barcodes.isNotEmpty()) {
                onInvalidQr()
            }
            imageProxy.close()
        }.addOnFailureListener {
            imageProxy.close()
        }
}
