package net.wardriver.mobile.data

/** Newline-separated list validation for WiGLE / policy text areas. */
object LineListValidators {
  const val MAX_SSID_BYTES = 32
  const val MAX_SSID_LINES = 32

  data class ValidationError(val lineNumber: Int, val line: String, val reason: String)

  /** Tagged pattern rules supported in the SSID watch list. */
  private val PATTERNS = arrayOf("PREFIX_LEN:", "PREFIX:", "CONTAINS:")

  /** Check if a line is a tagged pattern rule. */
  private fun isPatternRule(line: String): Boolean =
      PATTERNS.any { line.startsWith(it) }

  /** Validate a tagged pattern line; returns error reason or null. */
  private fun validatePatternLine(line: String): String? {
    if (line.startsWith("PREFIX_LEN:")) {
      val rest = line.removePrefix("PREFIX_LEN:")
      if (rest.isBlank()) return "PREFIX_LEN requires a prefix and length"
      // Must have a comma separating prefix and length.
      val lastComma = rest.lastIndexOf(',')
      if (lastComma < 1) return "PREFIX_LEN requires format: PREFIX,LENGTH"
      val lenStr = rest.substring(lastComma + 1).trim()
      if (lenStr.toIntOrNull() == null || lenStr.toInt() !in 1..32) {
        return "PREFIX_LEN length must be 1-32"
      }
    } else if (line.startsWith("PREFIX:")) {
      val rest = line.removePrefix("PREFIX:")
      if (rest.isBlank()) return "PREFIX requires a non-empty value"
    } else if (line.startsWith("CONTAINS:")) {
      val rest = line.removePrefix("CONTAINS:")
      if (rest.isBlank()) return "CONTAINS requires a non-empty value"
    }
    return null
  }

  /** Non-empty trimmed lines; max [MAX_SSID_BYTES] UTF-8 bytes for exact SSIDs; max [MAX_SSID_LINES] unique lines. */
  fun validateSsidLines(text: String): ValidationError? {
    val seen = linkedSetOf<String>()
    for ((index, raw) in text.lineSequence().withIndex()) {
      val line = raw.trim()
      if (line.isEmpty()) continue
      // Pattern rules have their own validation.
      if (isPatternRule(line)) {
        val patternErr = validatePatternLine(line)
        if (patternErr != null) {
          return ValidationError(lineNumber = index + 1, line = line, reason = patternErr)
        }
        if (seen.size >= MAX_SSID_LINES && line !in seen) {
          return ValidationError(
              lineNumber = index + 1, line = line, reason = "too_many"
          )
        }
        seen.add(line)
        continue
      }
      // Exact SSID — enforce byte limit.
      if (line.toByteArray(Charsets.UTF_8).size > MAX_SSID_BYTES) {
        return ValidationError(
            lineNumber = index + 1, line = line, reason = "too_long"
        )
      }
      if (seen.size >= MAX_SSID_LINES && line !in seen) {
        return ValidationError(
            lineNumber = index + 1, line = line, reason = "too_many"
        )
      }
      seen.add(line)
    }
    return null
  }

  /** User-facing validation message for [SavedMultilineField]. */
  fun validateSsidLinesMessage(text: String): String? {
    val err = validateSsidLines(text) ?: return null
    return when (err.reason) {
      "too_long" -> formatSsidTooLongMessage(err)
      "too_many" -> formatSsidTooManyMessage()
      else -> "${err.reason} (line ${err.lineNumber})"
    }
  }

  /** Trim lines, drop blanks, dedupe preserving first occurrence order. */
  fun normalizeSsidLines(text: String): String {
    val out = linkedSetOf<String>()
    for (raw in text.lineSequence()) {
      val line = raw.trim()
      if (line.isNotEmpty()) out.add(line)
    }
    return out.joinToString("\n")
  }

  fun formatSsidTooLongMessage(error: ValidationError): String =
      "Line ${error.lineNumber}: SSID must be at most $MAX_SSID_BYTES bytes"

  fun formatSsidTooManyMessage(): String =
      "At most $MAX_SSID_LINES unique entries (one per line)"
}
