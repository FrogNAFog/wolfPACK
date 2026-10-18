package net.wardriver.mobile.upload

import org.json.JSONObject

/** Match control firmware `UPLOAD_ERR_TEXT_MAX` for error snippets. */
internal const val UPLOAD_ERR_TEXT_MAX = 3072

private val WDGWARS_DUPLICATE_NEEDLES =
    listOf(
        "already upload",
        "byte-for-byte",
        "byte for byte",
        "same byte",
        "duplicate",
        "same content",
        "identical content",
    )

internal fun sanitizeUploadLogText(raw: String): String {
    val collapsed =
        raw
            .map { c ->
                when (c) {
                    '\n', '\r', '\t' -> ' '
                    else -> c
                }
            }.joinToString("")
    if (collapsed.length <= UPLOAD_ERR_TEXT_MAX) return collapsed
    return collapsed.take(UPLOAD_ERR_TEXT_MAX) + "... [truncated]"
}

private fun bodyJsonBool(
    body: String,
    field: String,
): Boolean? {
    return try {
        val json = JSONObject(body)
        if (!json.has(field)) {
            null
        } else {
            json.optBoolean(field, false)
        }
    } catch (_: Throwable) {
        when {
            body.contains("\"$field\":true") || body.contains("\"$field\": true") -> true
            body.contains("\"$field\":false") || body.contains("\"$field\": false") -> false
            else -> null
        }
    }
}

private fun bodyJsonString(
    body: String,
    field: String,
): String? {
    return try {
        val json = JSONObject(body)
        json.optString(field, "").ifBlank { null }
    } catch (_: Throwable) {
        null
    }
}

private fun jsonStrSuggestsDuplicate(text: String): Boolean {
    val lower = text.lowercase()
    return listOf("duplicate", "already", "uploaded", "conflict").any { lower.contains(it) }
}

private fun wdgwarsJsonOk(body: String): Boolean = bodyJsonBool(body, "ok") == true

private fun wdgwarsJsonDuplicate(body: String): Boolean {
    if (bodyJsonBool(body, "duplicate") == true) return true
    if (bodyJsonBool(body, "already_exists") == true) return true
    for (key in listOf("error", "code", "message")) {
        val value = bodyJsonString(body, key)
        if (value != null && jsonStrSuggestsDuplicate(value)) return true
    }
    return false
}

private fun wdgwarsBodySuggestsDuplicate(body: String): Boolean {
    if (body.isEmpty()) return false
    val lower = body.lowercase()
    return WDGWARS_DUPLICATE_NEEDLES.any { lower.contains(it) }
}

/** HTTP 409 with duplicate semantics — retry after a prior successful ingest. */
internal fun wdgwarsDuplicateAccepted(
    statusCode: Int,
    body: String,
): Boolean {
    if (statusCode != 409) return false
    if (wdgwarsBodySuggestsDuplicate(body)) return true
    return wdgwarsJsonDuplicate(body)
}

/**
 * True when wdgwars accepted the upload (aligned with PACK `wdgwars_upload_accepted`).
 */
internal fun wdgwarsUploadAccepted(
    statusCode: Int,
    body: String,
): Boolean {
    if (wdgwarsDuplicateAccepted(statusCode, body)) return true
    if (statusCode in 200..299) {
        val ok = bodyJsonBool(body, "ok")
        return when (ok) {
            true -> true
            false -> wdgwarsJsonDuplicate(body)
            null -> true
        }
    }
    if (statusCode in 500..599) {
        return wdgwarsJsonOk(body)
    }
    return false
}

/** WiGLE upload JSON uses top-level `"success": true` on HTTP 2xx. */
internal fun wigleUploadAccepted(
    statusCode: Int,
    body: String,
): Boolean {
    if (statusCode !in 200..299) return false
    return body.contains("\"success\":true") || body.contains("\"success\": true")
}

/**
 * True when [body] indicates upload success for [field] (`success` for WiGLE, `ok` for wdgwars).
 */
internal fun responseIndicatesSuccess(
    body: String,
    field: String,
): Boolean = bodyJsonBool(body, field) == true

internal fun validateWigleUploadResponse(
    statusCode: Int,
    body: String,
): Result<Unit> {
    if (!wigleUploadAccepted(statusCode, body)) {
        val snippet = sanitizeUploadLogText(body)
        val msg =
            if (statusCode !in 200..299) {
                "HTTP $statusCode: $snippet"
            } else if (snippet.isBlank()) {
                "WiGLE upload response did not indicate success"
            } else {
                "WiGLE upload response did not indicate success: $snippet"
            }
        return Result.failure(IllegalStateException(msg))
    }
    return Result.success(Unit)
}

internal fun validateWdgUploadResponse(
    statusCode: Int,
    body: String,
): Result<Unit> {
    if (wdgwarsUploadAccepted(statusCode, body)) {
        return Result.success(Unit)
    }
    val msg = "wdgwars HTTP $statusCode: ${sanitizeUploadLogText(body)}"
    return Result.failure(IllegalStateException(msg))
}
