package net.wardriver.mobile.policy

import net.wardriver.mobile.NativeCore
import org.json.JSONArray
import org.json.JSONObject

/** Serialize Android signed `Int` sniffer IDs as PackLink wire `u32` JSON numbers. */
internal fun snifferIdAsWireU32(snifferId: Int): Long = snifferId.toLong() and 0xFFFF_FFFFL

/** Builds PackLink `PolicySyncV1` wire bytes for gateway SoftAP write. */
object PolicySyncBuilder {
  data class RosterEntry(
      val snifferId: Int,
      val mac: String,
      val fleetSlot: Int = 0,
      /** wolfPACK role wire value: 0 WiFi, 1 BLE. */
      val roleWire: Int = 0,
  )

  data class GeofenceBounds(
      val latMin: Double,
      val latMax: Double,
      val lonMin: Double,
      val lonMax: Double,
  )

  /**
   * MVP policy: scan allowed, empty roster placeholder, optional geofence + denylists.
   * Packed via Rust `packPolicySyncJson` JNI (mirrors `wardriver-packlink`).
   */
  fun buildWireBytes(
      roster: List<RosterEntry> = emptyList(),
      geofence: GeofenceBounds? = null,
      denylistBssids: List<String> = emptyList(),
      denylistSsids: List<String> = emptyList(),
      detectionDisableWifiMask: Int = 0,
      detectionDisableBleMask: Int = 0,
      ssidWatch: List<String> = emptyList(),
      scanAllowed: Boolean = true,
      wardriveSuppressed: Boolean = false,
      dwellMs: Int = 200,
      channels: List<Int> = listOf(1, 6, 11),
      seq: Int = 0,
      bleActiveScan: Boolean = false,
  ): ByteArray? {
    val chanList = channels.distinct().sorted().take(32)
    val root =
        JSONObject().apply {
          put("scan_allowed", scanAllowed)
          put("wardrive_suppressed", wardriveSuppressed)
          put("geofence_enabled", geofence != null)
          put("geofence_lat_min", geofence?.latMin ?: 0.0)
          put("geofence_lat_max", geofence?.latMax ?: 0.0)
          put("geofence_lon_min", geofence?.lonMin ?: 0.0)
          put("geofence_lon_max", geofence?.lonMax ?: 0.0)
          put("denylist_bssids", JSONArray(denylistBssids))
          put("denylist_ssids", JSONArray(denylistSsids))
          put("detection_disable_wifi_mask", detectionDisableWifiMask)
          put("detection_disable_ble_mask", detectionDisableBleMask)
          put("ble_active_scan", bleActiveScan)
          put("ssid_watch", JSONArray(ssidWatch))
          put("dwell_ms", dwellMs.coerceIn(50, 10_000))
          put("chan_count", chanList.size)
          put("channels", JSONArray(chanList))
          put("seq", seq)
          put(
              "roster",
              JSONArray().apply {
                roster.forEach { e ->
                  put(
                      JSONObject().apply {
                        put("sniffer_id", snifferIdAsWireU32(e.snifferId))
                        put("mac", e.mac)
                        put("fleet_slot", e.fleetSlot)
                        put("role", e.roleWire.coerceIn(0, 1))
                      },
                  )
                }
              },
          )
        }
    return NativeCore.packPolicySync(root.toString())
  }
}
