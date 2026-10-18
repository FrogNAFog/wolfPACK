# Detection methods (`detection_method_id` reference)

This document is the **user-facing map** for the numeric **`detection_method_id`** field in Detection **`*.detectioncsv` v2** files (see [`DETECTIONCSV.md`](DETECTIONCSV.md) for full layout) and in Remote ID **`rid-*.PACKcsv`** sessions (see [`PACKCSV_REMOTE_ID.md`](PACKCSV_REMOTE_ID.md)). The same IDs appear in **`/api/stats`** as **`detection_last_detection_method`** (LPR-camera / mobile surveillance) and **`remote_id_last_detection_method`** (Remote ID) on the last alert snapshot.

**Maintainers:** Whenever you add a new detection path that appends rows to **`det-*.detectioncsv`**, **`Axon-<timestamp>.mdetectioncsv`**, **`rid-*.PACKcsv`**, or any future detection session file, **update this file** with the new ID, human-readable name, and one-line behavior note. Updates must be coordinated across PACK and wolfPACK and any ingestors.

Source of truth in code: [`DetectionWifiMethod` / `DetectionBleMethod`](../wardriver-core/src/detection.rs) and [`RemoteIdWifiDetectionMethod` / `RemoteIdBleDetectionMethod`](../wardriver-core/src/detection.rs) (`as_u8()`). Default disable masks match [`DETECTION_WIFI_DISABLE_MASK_DEFAULT` / `DETECTION_BLE_DISABLE_MASK_DEFAULT`](../wardriver-core/src/detection.rs) (deprecated methods off).

---

## LPR-camera (Flock Safety) methods

### WiFi methods (`1`–`3`)

WiFi LPR-camera and mobile-surveillance OUI methods run on **WiFi-role sniffers**. Qualifying hits are sent to the control node as **broadcast** ESP-NOW **`DetectionAlert`** frames (no `PeerJoin` or encrypted peer slot); primary detection stays on that path. Wardrive **`ProbeReqFrag`** and HTTP probe dumps carry follow-up telemetry only for MACs that raised **`DetectionAlert`** on the sniffer since boot. The control node **logs** alerts (Detection / mDetection CSV, OLED, stats) and does **not** re-run WiFi heuristics on probe follow-ups.

| ID | Label (UI / docs) | Status | Source data | Notes |
|----|-------------------|--------|-------------|--------|
| **1** | Wildcard probe + Flock OUI | `deprecated` | Sniffer promisc → `DetectionAlert` | Infrastructure OUI allowlist on the sniffer. **Off by default** (`detection_disable_wifi_mask` bit 0). |
| **2** | Wildcard probe + IE signature + Flock OUI | `implemented` | Same | Wildcard SSID plus Lite-On vendor anchor suffix (HT/VHT/WPA tail) + Flock OUI. **On by default**. |
| **3** | Wildcard probe + IE signature (any MAC) | `implemented` | Same | Lite-On anchor suffix without OUI requirement. **On by default**. |
| **6–31** | _(reserved)_ | `n/a` | — | Reserved for future methods. |

**Disable bitmask:** **`detection_disable_wifi_mask`**. For WiFi method **`m`** in `1..=31`, bit **`(m − 1)`** means “disable this method” when set (web UI: checked = enabled). The control node persists the mask on SD and pushes it to sniffers via **`ControlWelcome`** / **`ControlCmd`**.

### BLE methods (`32`–`34`)

BLE LPR-camera and mobile surveillance run on **BLE-role sniffers** from LE advertisement observations (NimBLE observer). Scan mode is **passive by default**; SoftAP `/detections` can enable **active BLE scan** (`ble_active_scan`), which sends SCAN_REQ so devices may return scan-response AD (name / manufacturer data). Axon / i-PRO methods (**35** / **36**) still match only the advertiser MAC OUI — active scan does not resolve BLE privacy addresses (RPA / NRPA). ESP32-C5 has **no Bluetooth Classic** (BR/EDR); WiGLE rows typed Classic BT are not visible to this fleet.

Qualifying hits are sent to the control node as **broadcast** ESP-NOW **`DetectionBleAlert`** frames (no `PeerJoin` or encrypted peer slot); primary detection stays on that path. Wardrive **`BleScanFrag`** and HTTP BLE dumps carry follow-up telemetry only for MACs that raised **`DetectionBleAlert`** on the sniffer since boot (or already appear in the control alert registry). The control node **logs** immediate alerts (Detection / mDetection CSV, OLED, stats) and does **not** re-run primary BLE heuristics on bulk follow-ups for those MACs.

| ID | Label (UI / docs) | Status | Source data | Notes |
|----|-------------------|--------|-------------|--------|
| **32** | Xuntong manufacturer ID | `deprecated` | Sniffer GAP observer → `DetectionBleAlert` | Company ID **0x09C8**. **Off by default**. |
| **33** | Advertised name keyword | `deprecated` | Same | Case-insensitive substrings. **Off by default**. |
| **34** | MAC OUI (infrastructure) | `deprecated` | Advertiser MAC | Flock infrastructure OUI list. **Off by default**. |

**Disable bitmask:** **`detection_disable_ble_mask`**. For BLE method **`m`** in `32..=63`, bit **`(m − 32)`** means “disable this method” when set.

**Scan mode:** **`ble_active_scan`** in SoftAP preferences / `Ping` / `ControlWelcome` (default off = passive).

### Related `/api/stats` fields (detection)

| Key | Meaning |
|-----|---------|
| **`detection_alerts_fired`** | Count of alerts counted as **WiFi signal path** (stats aggregate). |
| **`ble_alerts_fired`** | Count of alerts counted as **BLE signal path**. |
| **`detection_followup_rows_written`** | Detection CSV rows written for MACs that **already alerted**, using the same time/distance throttle as wardriving WiGLE rows (trilateration). |
| **`detection_alerted_macs_tracked`** | Gauge: WiFi + BLE MACs currently in the follow-up registry (LRU-capped). |
| **`detection_last_signal_kind`** | `0` = last alert WiFi-side, `1` = BLE-side. |
| **`detection_last_detection_method`** | Same numeric IDs as **`detection_method_id`** above. |
| **`detection_disable_wifi_mask`** / **`detection_disable_ble_mask`** | Per-method disable bitmasks (persisted in `wolfPACK/config/preferences.json`). |
| **`ble_active_scan`** | SoftAP preference: BLE-role sniffers use active LE scan when true (default false = passive). |

---

## Mobile surveillance (Axon / i-PRO) methods

Mobile surveillance detectors share the LPR-camera method ID spaces: **Wi-Fi methods `4`–`5`** and **BLE methods `35`–`36`**. Disable bits live in **`detection_disable_wifi_mask`** (bit `(method_id − 1)`) and **`detection_disable_ble_mask`** (bit `(method_id − 32)`). Persisted in `wolfPACK/config/preferences.json`. The detections page checkbox **checked** means the method is enabled (bit clear).

Source of truth: [`is_mobile_surveillance_method`](../wardriver-core/src/detection.rs), OUI helpers in [`lpr_camera_ble.rs`](../wardriver-core/src/lpr_camera_ble.rs), Wi-Fi [`LprCameraWifiDetector::eval_mac`](../wardriver-core/src/lpr_camera_wifi.rs), and BLE [`LprCameraBleDetector::ingest`](../wardriver-core/src/lpr_camera_ble.rs).

PackLink / Room event kinds are **`DETECTION_WIFI`** / **`DETECTION_BLE`**; user-visible titles use the method ID (`Axon` / `i-PRO`).

### Wi-Fi methods (`4`–`5`)

| ID | Label (UI / docs) | Status | Source data | Notes |
|----|-------------------|--------|-------------|--------|
| **4** | Axon OUI (`00:25:DF`) | `experimental` | Beacon / probe-response BSSID or probe-request source | Mobile surveillance Wi-Fi. Rows go to **`Axon-*.mdetectioncsv`**. **On by default**. |
| **5** | i-PRO OUI (`D4:2D:C5`) | `experimental` | Probe-request source only | Mobile surveillance Wi-Fi. Rows go to **`Axon-*.mdetectioncsv`**. **On by default**. |

#### Method **4** — Axon OUI (Wi-Fi)

| Field | Value |
|-------|--------|
| **ID** | **4** (`DetectionWifiMethod::AxonOui`) |
| **Label** | Axon OUI (Wi-Fi) |
| **Rule** | IEEE address prefix **`00:25:DF`** on beacon / probe-response BSSID or probe-request source |
| **Source data** | WiFi-role sniffer promisc → `DetectionAlert` |
| **Alerts** | OLED/buzzer path **AXON** (`AlertKind::Surveillance`); counter **`mobile_surveillance_alerts_fired`** |
| **SD log** | `mobile_surveillance/detections/Axon-<timestamp>.mdetectioncsv` (`Detection-mdetectioncsv-1.0`, row type `DETECTION-WIFI`) |
| **Default** | **On** |

#### Method **5** — i-PRO OUI (Wi-Fi)

| Field | Value |
|-------|--------|
| **ID** | **5** (`DetectionWifiMethod::IproOui`) |
| **Label** | i-PRO OUI (Wi-Fi) |
| **Rule** | IEEE address prefix **`D4:2D:C5`** on probe-request source only (not beacon / probe-response BSSID) |
| **Source data** | WiFi-role sniffer promisc → `DetectionAlert` |
| **Alerts** | OLED/buzzer path **I-PRO** (`AlertKind::Surveillance`); counter **`mobile_surveillance_alerts_fired`** |
| **SD log** | Same `Axon-*.mdetectioncsv` session; row type `DETECTION-WIFI` |
| **Default** | **On** |

### BLE methods (`35`–`36`)

#### Method **35** — Axon OUI (BLE)

| Field | Value |
|-------|--------|
| **ID** | **35** (PACK `FlockBleDetectionMethod::AxonOui`) |
| **Label** | Axon OUI |
| **Rule** | Advertiser IEEE address prefix **`00:25:DF`** (first three octets) |
| **Source data** | `BleScanFrag` batches from BLE-role sniffers; merged on the control node |
| **Alerts** | OLED/buzzer path **AXON**; counter **`mobile_surveillance_alerts_fired`** |
| **SD log** | `mobile_surveillance/detections/Axon-<timestamp>.mdetectioncsv` (`Detection-mdetectioncsv-1.0`, row type `DETECTION-BLE`) |
| **Default** | **On** (PACK default BLE mask: methods 32–34 off, 35–36 on) |

#### Method **36** — i-PRO OUI (BLE)

| Field | Value |
|-------|--------|
| **ID** | **36** (`DetectionBleMethod::IproOui`) |
| **Label** | i-PRO OUI |
| **Rule** | Advertiser IEEE address prefix **`D4:2D:C5`** |
| **Source data** | Same BLE path as method 35 |
| **Alerts** | OLED/buzzer path **I-PRO**; counter **`mobile_surveillance_alerts_fired`** |
| **SD log** | Same `Axon-*.mdetectioncsv` session; row type `DETECTION-BLE` |
| **Default** | **On** |

**Related `/api/stats` keys:** `mobile_surveillance_alerts_fired`, `mobile_surveillance_last_src`, `mobile_surveillance_last_seen_ms`, `mobile_surveillance_last_sniffer_id`, `mobile_surveillance_last_detection_method` (4/5/35/36), nested `mobile_surveillance_last`.

---

## Remote ID methods

This section maps numeric **`detection_method_id`** values in Remote ID **`rid-*.PACKcsv`** sessions (see [`PACKCSV_REMOTE_ID.md`](PACKCSV_REMOTE_ID.md)). The same IDs appear in `/api/stats` as **`remote_id_last_detection_method`**.

Source of truth in code: [`RemoteIdWifiDetectionMethod` / `RemoteIdBleDetectionMethod`](../wardriver-core/src/detection.rs) (`as_u8()`).

### Wi-Fi methods (`64`–`95`)

Wi-Fi Remote ID runs on **WiFi-role sniffers** when the dwell channel is **6** (2.4 GHz) or **149** (5 GHz). Qualifying hits are sent to the control node as broadcast ESP-NOW **`RemoteIdAlert`** frames.

| ID | Label | Transport | Notes |
|----|-------|-----------|-------|
| **64** | Beacon vendor IE | `beacon` | OpenDroneID vendor IE OUI `FA:0B:BC` (and compatible layouts) |
| **65** | NAN action | `nan` | NAN destination `51:6f:9a:01:00:00` action frames |

**Disable bitmask:** `remote_id_disable_mask`. For Wi-Fi method **`m`** in `64..=95`, bit **`(m − 64)`** disables that method when set.

### BLE methods (`96`–`127`)

BLE Remote ID runs on **BLE-role sniffers** from LE advertisement observations (passive by default; SoftAP `/detections` can enable active BLE scan).

| ID | Label | Notes |
|----|-------|-------|
| **96** | Service Data `0xFFFA` | ASTM UUID + app code `0x0D` |
| **97** | Legacy manufacturer `0x0002` | Intel namespace + app code `0x0D` |

**Disable bitmask:** `remote_id_disable_mask`. For BLE method **`m`** in `96..=127`, bit **`(m − 96)`** disables that method when set.

### Related `/api/stats` fields (Remote ID)

| Key | Meaning |
|-----|---------|
| `remote_id_alerts_rx` | `FrameType::RemoteIdAlert` frames accepted on control |
| `remote_id_rows_written` | PACKcsv rows appended on SD |
| `remote_id_last_src` | Aircraft MAC on the last alert |
| `remote_id_last_signal_kind` | `0` = Wi-Fi, `1` = BLE |
| `remote_id_last_detection_method` | Same numeric IDs as **`detection_method_id`** above |
| `remote_id_disable_mask` | Per-method disable bitmask (persisted in preferences) |
