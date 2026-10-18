package net.wardriver.mobile.upload

import android.util.Log
import okhttp3.Credentials
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

internal fun uploadHttpClient(): OkHttpClient =
    OkHttpClient.Builder()
        .connectTimeout(30, TimeUnit.SECONDS)
        .readTimeout(UPLOAD_HTTP_TIMEOUT_SEC, TimeUnit.SECONDS)
        .writeTimeout(UPLOAD_HTTP_TIMEOUT_SEC, TimeUnit.SECONDS)
        .build()

internal fun postWigleCsv(
    client: OkHttpClient,
    apiName: String,
    apiToken: String,
    csvBytes: ByteArray,
    fileName: String,
): Result<Unit> {
    val multipart =
        MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart(
                "file",
                fileName,
                csvBytes.toRequestBody("text/csv".toMediaType()),
            )
            .build()
    val auth = Credentials.basic(apiName, apiToken)
    val request =
        Request.Builder()
            .url(WIGLE_UPLOAD_URL)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .post(multipart)
            .build()
    return try {
        client.newCall(request).execute().use { resp ->
            val respBody = resp.body?.string().orEmpty()
            validateWigleUploadResponse(resp.code, respBody)
                .also { logUploadFailure("WiGLE", it) }
        }
    } catch (e: Exception) {
        Result.failure(e)
    }
}

internal fun postWdgCsv(
    client: OkHttpClient,
    apiKey: String,
    csvBytes: ByteArray,
    fileName: String,
): Result<Unit> {
    val multipart =
        MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart(
                "file",
                fileName,
                csvBytes.toRequestBody("text/csv".toMediaType()),
            )
            .build()
    val request =
        Request.Builder()
            .url(WDG_UPLOAD_URL)
            .header("X-API-Key", apiKey)
            .header("User-Agent", WDG_USER_AGENT)
            .header("Accept", "application/json")
            .post(multipart)
            .build()
    return try {
        client.newCall(request).execute().use { resp ->
            val respBody = resp.body?.string().orEmpty()
            validateWdgUploadResponse(resp.code, respBody)
                .also { logUploadFailure("wdgwars", it) }
        }
    } catch (e: Exception) {
        Result.failure(e)
    }
}

internal const val WIGLE_UPLOAD_URL = "https://api.wigle.net/api/v2/file/upload"
internal const val WDG_UPLOAD_URL = "https://wdgwars.pl/api/upload-csv"
internal const val WDG_USER_AGENT = "wolfPACK/0.1 (+wdgwars)"

private const val TAG = "wardriver-upload"

private fun logUploadFailure(
    destLabel: String,
    result: Result<Unit>,
) {
    if (result.isFailure) {
        Log.w(TAG, "$destLabel upload failed: ${result.exceptionOrNull()?.message}")
    }
}

internal fun logUploadMarkFailure(error: Throwable) {
    Log.e(
        TAG,
        "upload accepted by server but local mark failed — do not retry without support",
        error,
    )
}
