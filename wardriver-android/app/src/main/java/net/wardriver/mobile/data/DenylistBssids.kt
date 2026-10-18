package net.wardriver.mobile.data

/** WiGLE BSSID denylist validation, normalization, and gateway SoftAP merge. */
object DenylistBssids {
    private val COLON_MAC_REGEX = Regex("^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$")

    /**
     * Validate newline-separated MAC lines.
     * Empty lines are ignored. Each non-empty line must be colon-separated hex octets.
     */
    fun validateMacLines(text: String): LineListValidators.ValidationError? {
        for ((index, raw) in text.lineSequence().withIndex()) {
            val line = raw.trim()
            if (line.isEmpty()) continue
            if (!COLON_MAC_REGEX.matches(line)) {
                return LineListValidators.ValidationError(lineNumber = index + 1, line = line, reason = "Invalid MAC format; expected AA:BB:CC:DD:EE:FF")
            }
        }
        return null
    }

    /** Normalize saved text: uppercase colon MACs, one per line, blank lines dropped. */
    fun normalizeMacLines(text: String): String =
        text
            .lineSequence()
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .map { line ->
                FleetRepository.normalizeMac(line)
                    ?: throw IllegalArgumentException("invalid MAC line: $line")
            }
            .joinToString("\n")

    /**
     * Manual denylist lines plus gateway SoftAP MAC (prepended, deduped).
     * Gateway MAC is normalized to `AA:BB:CC:DD:EE:FF`.
     */
    fun mergedManualPlusGateway(manualLines: List<String>, gatewayMac: String?): List<String> {
        val out = linkedSetOf<String>()
        FleetRepository.normalizeMac(gatewayMac.orEmpty())?.let { out.add(it) }
        for (line in manualLines) {
            FleetRepository.normalizeMac(line)?.let { out.add(it) }
        }
        return out.toList()
    }

    fun formatValidationMessage(error: LineListValidators.ValidationError): String =
        "Line ${error.lineNumber}: use colon format AA:BB:CC:DD:EE:FF (one MAC per line)"
}
