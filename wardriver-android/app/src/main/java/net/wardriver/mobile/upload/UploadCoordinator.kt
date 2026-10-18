package net.wardriver.mobile.upload

import android.content.Context
import android.content.Intent
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.withTimeoutOrNull
import net.wardriver.mobile.service.WardriveSessionService
import java.util.concurrent.atomic.AtomicReference

/** Global upload-run state shared between WorkManager and wardrive/catch-up ingest. */
object UploadCoordinator {
    @Volatile
    var uploadInProgress: Boolean = false

    @Volatile
    var resumeWardriveAfterUpload: Boolean = false

    @Volatile
    var catchUpWasActiveBeforeUpload: Boolean = false

    private val prepDeferred = AtomicReference<CompletableDeferred<Boolean>?>(null)

/** Wait for upload prep; null when timed out. */
suspend fun awaitPrepareForUpload(
    context: Context,
    timeoutMs: Long = 15_000L,
): Boolean? {
    val deferred = CompletableDeferred<Boolean>()
    prepDeferred.set(deferred)
    context.startService(
        Intent(context, WardriveSessionService::class.java)
            .setAction(WardriveSessionService.ACTION_PREPARE_FOR_UPLOAD),
    )
    val result = withTimeoutOrNull(timeoutMs) { deferred.await() }
    prepDeferred.compareAndSet(deferred, null)
    return result
}

    fun completePrepare(resumeWardrive: Boolean) {
        prepDeferred.getAndSet(null)?.complete(resumeWardrive)
    }

    fun failPrepare() {
        prepDeferred.getAndSet(null)?.complete(false)
    }

    fun requestResumeAfterUpload(context: Context) {
        context.startService(
            Intent(context, WardriveSessionService::class.java)
                .setAction(WardriveSessionService.ACTION_RESUME_AFTER_UPLOAD),
        )
    }
}
