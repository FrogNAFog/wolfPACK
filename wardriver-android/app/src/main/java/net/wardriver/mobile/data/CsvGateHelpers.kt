package net.wardriver.mobile.data

/** CSV gate blocker labels and home-zone predicate shared by dashboard stats and unit tests. */
object CsvGateHelpers {
    fun csvInsideHome(
        sessionActive: Boolean,
        latchSuppressed: Boolean,
        gpsFixOk: Boolean,
        pointInCircle: Boolean,
    ): Boolean {
        if (sessionActive) return latchSuppressed
        return gpsFixOk && pointInCircle
    }

    fun csvGateBlockers(
        scanAllowed: Boolean,
        clockOk: Boolean,
        gpsFixOk: Boolean,
        insideHome: Boolean,
    ): List<String> {
        val blockers = mutableListOf<String>()
        if (!scanAllowed) blockers.add("Paused")
        if (!clockOk) blockers.add("Phone time unset")
        if (insideHome) blockers.add("Inside home zone")
        if (!gpsFixOk) blockers.add("GPS not ready")
        return blockers
    }
}
