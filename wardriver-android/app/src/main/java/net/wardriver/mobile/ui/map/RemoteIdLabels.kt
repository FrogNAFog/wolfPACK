package net.wardriver.mobile.ui.map

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** ASTM F3411 / OpenDroneID labels for Remote ID detail UI. */
fun remoteIdBasicIdTypeLabel(idType: Int): String =
    when (idType) {
        0 -> "None"
        1 -> "Serial Number"
        2 -> "CAA Registration ID"
        3 -> "UTM Assigned ID"
        4 -> "Specific Session ID"
        else -> "Type $idType"
    }

fun remoteIdUaTypeLabel(uaType: Int): String =
    when (uaType) {
        0 -> "None"
        1 -> "Aeroplane"
        2 -> "Helicopter"
        3 -> "Gyroplane"
        4 -> "Hybrid Lift"
        5 -> "Ornithopter"
        6 -> "Glider"
        7 -> "Kite"
        8 -> "Free Balloon"
        9 -> "Captive Balloon"
        10 -> "Airship"
        11 -> "Free Fall / Parachute"
        12 -> "Rocket"
        13 -> "Tethered Powered Aircraft"
        14 -> "Ground Obstacle"
        15 -> "Other"
        else -> "Type $uaType"
    }

fun remoteIdLocationStatusLabel(status: Int): String =
    when (status) {
        0 -> "Undeclared"
        1 -> "Ground"
        2 -> "Airborne"
        3 -> "Emergency"
        4 -> "Remote ID System Failure"
        else -> "Status $status"
    }

fun remoteIdOperatorLocationTypeLabel(type: Int): String =
    when (type) {
        0 -> "Takeoff"
        1 -> "Live GNSS"
        2 -> "Fixed"
        else -> "Type $type"
    }

fun remoteIdOperatorIdTypeLabel(idType: Int): String =
    when (idType) {
        0 -> "None"
        1 -> "CAA"
        else -> "Type $idType"
    }

fun remoteIdDetectionMethodLabel(methodId: Int): String =
    when (methodId) {
        64 -> "Beacon vendor IE"
        65 -> "NAN action"
        96 -> "Service Data 0xFFFA"
        97 -> "Legacy manufacturer 0x0002"
        else -> "Method $methodId"
    }

fun remoteIdAccuracyLabel(nibble: Int): String =
    if (nibble == 0) {
        "Unknown"
    } else {
        "Level $nibble"
    }

/** Epoch ms on or after 2020-01-01 UTC. */
private const val PLAUSIBLE_WALL_UNIX_MS_MIN = 1_577_836_800_000L

/** Max skew ahead of device now for a plausible wall timestamp. */
private const val PLAUSIBLE_WALL_UNIX_MS_AHEAD = 60_000L

/** True when [ms] looks like a real Unix wall clock, not boot-monotonic. */
fun isPlausibleWallUnixMs(
    ms: Long,
    nowMs: Long = System.currentTimeMillis(),
): Boolean = ms >= PLAUSIBLE_WALL_UNIX_MS_MIN && ms <= nowMs + PLAUSIBLE_WALL_UNIX_MS_AHEAD

fun formatRemoteIdDetectionTime(unixMs: Long): String {
    if (!isPlausibleWallUnixMs(unixMs)) {
        return "Time not synced"
    }
    val formatter =
        DateTimeFormatter.ofPattern("MMM d, yyyy h:mm:ss a")
            .withZone(ZoneId.systemDefault())
    return formatter.format(Instant.ofEpochMilli(unixMs))
}

fun formatOptionalDouble(value: Double?, suffix: String = ""): String? =
    value?.let { "%.6f$suffix".format(it) }

fun formatOptionalFloat(value: Float?, suffix: String = ""): String? =
    value?.let { "%.1f$suffix".format(it) }
