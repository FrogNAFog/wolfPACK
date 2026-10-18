# Remote ID `PACKcsv`

wolfPACK writes Remote ID detection sessions on the control node as **`rid-*.PACKcsv`** files under `wolfPACK/remote_id/detections/`. Rows use WiGLE CSV v1.6 column names and order, plus OpenDroneID extension fields. Detection sessions remain **`det-*.detectioncsv`**.

## Location

- Root: `/sdcard/wolfPACK/` (see [`storage::sd_data_root`](../wardriver-control/src/storage/mod.rs)).
- Sessions: `remote_id/detections/rid-<timestamp>.PACKcsv`.

## File layout

1. **Line 1 — pre-header:** starts with `PACK-remote-id-1.1`, then comma-separated `key=value` metadata (`appRelease`, `model`, `device`, `board`, `brand=wolfPACK`, …).
2. **Line 2 — header:** [`csv::PACK_REMOTE_ID_HEADER`](../wardriver-control/src/csv.rs) = WiGLE v1.6 [`CSV_HEADER`](../wardriver-control/src/csv.rs) + Remote ID extension columns.
3. **Data rows:** one row per Remote ID alert (and optional geo-throttled follow-up rows for tracked aircraft MACs).

## WiGLE columns (summary)

Same semantics as WiGLE v1.6 WiFi rows:

- **`MAC`** — aircraft source MAC (Wi-Fi transmitter or BLE advertiser).
- **`SSID`**, **`AuthMode`** — usually empty.
- **`FirstSeen`**, **`Channel`**, **`Frequency`**, **`RSSI`**
- **`CurrentLatitude`**, **`CurrentLongitude`**, **`AltitudeMeters`**, **`AccuracyMeters`** — control-node GPS (observer position).
- **`RCOIs`**, **`MfgrId`** — usually empty.
- **`Type`** — `REMOTE-ID-WIFI` or `REMOTE-ID-BLE`.

## Extension columns (after `Type`)

| Column | Meaning |
|--------|---------|
| `uas_id` | Decoded Basic ID bytes (ASCII, null-trimmed) |
| `id_type` | Basic ID type nibble when present on wire |
| `ua_type` | UA type nibble when present on wire |
| `status` | Location status nibble when present on wire |
| `drone_lat`, `drone_lon` | Aircraft position from the broadcast |
| `alt_geo_m` | Geodetic altitude (meters) |
| `heading_deg` | Horizontal direction (degrees) |
| `speed_h_mps` | Horizontal speed (m/s) |
| `operator_id` | Operator registration string when present on wire |
| `operator_lat`, `operator_lon` | Operator location from System message |
| `detection_method_id` | Numeric method ID — see [`DETECTION_METHODS.md`](DETECTION_METHODS.md) |
| `signal_kind` | `0` = Wi-Fi, `1` = BLE |
| `wifi_transport` | `beacon`, `nan`, or empty (BLE rows) |
| `height_agl_m` | Height above ground (meters) |
| `height_type` | Height reference (`0` = above takeoff) |
| `horiz_accuracy` | Horizontal accuracy nibble |
| `vert_accuracy` | Vertical accuracy nibble |
| `speed_accuracy` | Speed accuracy nibble |
| `baro_accuracy` | Barometric altitude accuracy nibble |
| `ts_accuracy` | Timestamp accuracy nibble |
| `timestamp_sec` | Location timestamp (seconds since the hour) |
| `operator_location_type` | Operator location type (`0` takeoff, `1` live GNSS, `2` fixed) |
| `operator_id_type` | Operator ID type when present on wire |

## Follow-up rows

After a MAC has produced at least one alert, the control node may append additional rows while the aircraft remains tracked, using the same 50 m geo throttle as wardriving WiGLE CSV (`MacTable` `REMOTE_ID` lane).

## Detection methods

Human-readable method IDs: [`DETECTION_METHODS.md`](DETECTION_METHODS.md).

Disable bitmask: `remote_id_disable_mask` in `wolfPACK/config/preferences.json` (persisted and pushed to sniffers via `ControlWelcome` / `ControlCmd`).
