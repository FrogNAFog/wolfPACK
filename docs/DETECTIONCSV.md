# Detection `detectioncsv` v2

wolfPACK writes Detection alert sessions on the control node as **`det-*.detectioncsv`** files (default prefix `det`). Mobile surveillance (Axon / i-PRO OUI methods **4**, **5**, **35**, **36**) writes under **`mobile_surveillance/detections/Axon-*.mdetectioncsv`** with the same WiGLE-prefixed schema. Rows use **WiGLE CSV v1.6 column names and order**, plus trailing Detection extension fields. Wardrive WiGLE captures remain under `wolfPACK/wigle/pending/` (see [`README.md`](../README.md)).

Column layout matches Linux **PACK** (`deflock_csv.rs` / `docs/DETECTIONCSV.md` in the PACK repository).

## Location

- Root: `/sdcard/wolfPACK/` (see [`storage::sd_data_root`](../wardriver-control/src/storage/mod.rs)).
- Detection sessions: `flock/detections/det-<timestamp>.detectioncsv`.
- Mobile surveillance sessions: `mobile_surveillance/detections/Axon-<timestamp>.mdetectioncsv`.

## File layout

1. **Line 1 — pre-header:** starts with `Detection-detectioncsv-2.0` (or `Detection-mdetectioncsv-1.0` for mobile surveillance), then comma-separated `key=value` metadata (`appRelease`, `model`, `device`, `board`, `brand=wolfPACK`, …). Same style as WiGLE pre-headers but a different format token.
2. **Line 2 — header:** [`csv::DETECTIONCSV_HEADER`](../wardriver-control/src/csv.rs) = [`csv::CSV_HEADER`](../wardriver-control/src/csv.rs) + `,detection_method_id,signal_kind,rssi_min,rssi_max,rssi_avg_q8,wildcard_count,distinct_ch`.
3. **Data rows:** one row per alert (and optional follow-up rows for trilateration — see below).

## WiGLE columns (summary)

Same semantics as WiGLE v1.6 WiFi/BLE rows:

- **`MAC`**, **`SSID`**, **`AuthMode`**, **`FirstSeen`**, **`Channel`**, **`Frequency`**, **`RSSI`**, **`CurrentLatitude`**, **`CurrentLongitude`**, **`AltitudeMeters`**, **`AccuracyMeters`**, **`RCOIs`**, **`MfgrId`**, **`Type`**

### `Type` values (Detection-specific)

| `Type` | Meaning |
|--------|---------|
| `DETECTION-WIFI` | Wi-Fi detection (LPR-camera methods 1–3 and mobile-surveillance OUI methods 4–5) |
| `DETECTION-BLE` | BLE detection (LPR-camera methods 32–34 and mobile-surveillance OUI methods 35–36) |

## Extension columns (after `Type`)

- **`detection_method_id`** — numeric ID from [`DetectionWifiMethod` / `DetectionBleMethod`](../wardriver-core/src/detection.rs) (`as_u8()`): WiFi **1–5**, BLE **32–36**. **Human-readable table:** [`DETECTION_METHODS.md`](DETECTION_METHODS.md).
- **`signal_kind`** — `0` = WiFi, `1` = BLE.
- **`rssi_min`**, **`rssi_max`**, **`rssi_avg_q8`**, **`wildcard_count`**, **`distinct_ch`** — burst / window stats for WiFi detection; BLE and OUI-only rows often repeat instantaneous RSSI.

## Follow-up rows

After a MAC has produced at least one alert, the control node may append **additional** rows for that MAC while it remains in range (same throttle as wardriving WiGLE CSV). Those rows reuse the **original** `detection_method_id` / `signal_kind` from the first alert; `FirstSeen` reflects each observation. Mobile-surveillance Wi-Fi also follows up on harvested AP BSSIDs after the initial OUI alert.

