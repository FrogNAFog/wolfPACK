# Mobile control app (Android)

Native Kotlin/Compose wardrive client for wolfPACK. The phone is the control brain; a gateway ESP32 handles ESP-NOW realtime duties and speaks PackLink over **gateway SoftAP + plaintext TCP**.

Wire format: [`PACKLINK_SOFTAP.md`](PACKLINK_SOFTAP.md). Gateway firmware: [`GATEWAY.md`](GATEWAY.md).

## Stack

| Layer | Technology |
|-------|------------|
| UI | Jetpack Compose |
| Gateway link | Gateway SoftAP + TCP 4710 + TOFU |
| GPS | Fused Location Provider + `GPS_PROVIDER` wake/fallback |
| DB | Room (SQLite) |
| Map | OSMDroid (offline tiles) |
| Rust core | `wardriver-mobile-core` via JNI |
| Upload | WorkManager + OkHttp |

Project root: [`wardriver-android/`](../wardriver-android/).

## SQLite schema

### `observation` (primary, append-only)

One row per **allowed** sighting after dedup + CSV gates. Re-visiting the same BSSID beyond 50 m produces **multiple** rows (matches legacy control `MacTable` semantics).

| Column | Type | Notes |
|--------|------|-------|
| `_id` | INTEGER PK | Auto-increment |
| `session_id` | INTEGER | Wardrive session |
| `sniffer_id` | INTEGER | Source sniffer |
| `observed_unix_ms` | INTEGER | Anchor-matched time |
| `lat`, `lon` | REAL | GPS at observation |
| `altitude_m`, `accuracy_m` | INT/REAL | WiGLE columns |
| `signal_kind` | TEXT | `WIFI` or `BLE` |
| `bssid` | TEXT | MAC |
| `ssid` | TEXT | Network name |
| `channel`, `frequency_mhz`, `rssi` | INT | RF |
| `auth_mode` | TEXT | WiGLE caps string |
| `uploaded` | BOOL | Incremental upload marker |

Entity: [`ObservationEntity.kt`](../wardriver-android/app/src/main/java/net/wardriver/mobile/db/WardriverDatabase.kt).

### `network_summary` (optional UI cache)

One row per BSSID — last seen SSID/RSSI/position. **Not** the dedup store; derived from observations.

### `route`

Phone GPS track points (polyline source). Logged ~every 3.8 m / 3 s when accuracy &lt; 25 m (WiGLE `MIN_ROUTE_*` pattern). Wardrive registers fused `HIGH_ACCURACY` (without waiting for an accurate fix before callbacks) and `GPS_PROVIDER` updates before SoftAP connect so GNSS wakes early; coarse hunting updates can refresh the dashboard while usable fixes (accuracy ≤ 25 m) append to the GPS timeline and route for sniffer time-matching. `GPS_PROVIDER` supplies position when fused stays coarse. While a wardrive session is active, the dashboard GPS line uses only that wardrive stream (or **Waiting for GPS** when none has arrived yet), not idle BALANCED one-shots. Wardrive stop removes fused and `GPS_PROVIDER` listeners before the service ends; the next start re-registers after a clean stop. Dashboard GPS detail appends `stale Ns` when the displayed fix is older than 3 s.

### `session`

Wardrive metadata: start/end, gateway MAC, geofence square bounds.

### `detection`

Detection / SSID-watch / geofence events (parallel to control `flock/detections/` on SD).

## Ingest gates

Ported from control via [`wardriver-mobile-core/src/enrichment.rs`](../wardriver-mobile-core/src/enrichment.rs):

1. `csv_enqueue_allowed` — trusted clock, GPS usable, outside home geofence, scan not held
2. BSSID / SSID denylists
3. `MacTable` 50 m revisit radius ([`REVISIT_RADIUS_M`](../wardriver-mobile-core/src/enrichment.rs))
4. GPS anchor match ([`gps_anchor_mobile`](../wardriver-core/src/gps_anchor_mobile.rs)) — native fused `accuracy_m` from Android, not HDOP

**Enrichment session:** at wardrive start, Kotlin opens a Rust session (`enrichmentSessionOpenJson`) with an empty per-run `MacTable` / `CellTable` (not seeded from `network_summary`). WiFi, BLE, and cell enrich calls share that session state and `next_id` until `enrichmentSessionCloseJson` on ingestor shutdown. Bulk PackLink ingest (`ingestObservationBulkJson`) and batch cell ingest (`ingestCellsJson`) require an open session.

**Cell tower path:** [`CellTowerCollector.kt`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ingest/CellTowerCollector.kt) reads Android `TelephonyManager` APIs; Rust applies gates and 50 m dedup (`enrich_cell`, `cell_table.rs`). Cell keys in `observation.bssid` omit colons (e.g. `310410_1_2`). `signal_kind` is `LTE` / `NR` / `GSM` / `WCDMA` / `CDMA`. Throttled to 20 s per GPS fix in [`WardriveSessionService`](../wardriver-android/app/src/main/java/net/wardriver/mobile/service/WardriveSessionService.kt).

**Fast mode:** when ingest queue &gt; 75%, skip MACs that would fail dedup anyway — never collapse multiple observations into one row.

## WiGLE prior-art mapping

Reference: [wiglenet/wigle-wifi-wardriving](https://github.com/wiglenet/wigle-wifi-wardriving)

| wolfPACK mobile | WiGLE Android | Notes |
|-----------------|---------------|-------|
| `observation` table | `location` table | wolfPACK stores gated observations only; no separate `network` dedup split |
| `WardriveSessionService` | `WigleService` | Foreground notification + pause/upload intents |
| GPS anchor + lerp | `DatabaseHelper.recoverLocations` + `GNSSListener` | Brief fix loss queues rows |
| `ObservationIngestor.uploadPending` | `ObservationUploader` | Cursor `_id > marker` |
| `MapScreen` polyline + markers | `AbstractMappingFragment` | Clustering at scale |
| CSV export | WiGLE CSV 1.6 via [`wigle.rs`](../wardriver-core/src/wigle.rs) | Secondary to SQLite |
| Cell towers | `TelephonyManager.getAllCellInfo` → `observation` → WiGLE cell CSV | Phone only; gateway PackLink path unchanged |

## Build (debug APK)

From [`wardriver-android/`](../wardriver-android/):

```bash
cd wardriver-android && ./gradlew :app:assembleDebug
```

Output: `wardriver-android/app/build/outputs/apk/debug/app-debug.apk`.

The `cargoBuildMobileCore` Gradle task runs before `preBuild` and invokes `cargo ndk` for `arm64-v8a`. Prerequisites: `rustup target add aarch64-linux-android`, `cargo install cargo-ndk`, Android NDK via SDK Manager.

## Connect gateway

Join the phone to the wolfPACK gateway management SoftAP and complete TOFU on first connect (see [`PACKLINK_SOFTAP.md`](PACKLINK_SOFTAP.md)). Cellular stays default for internet; PackLink uses the gateway Wi-Fi network only.

| Entry point | Action |
|-------------|--------|
| Dashboard → **Gateway** card | **Connect gateway** (no trust yet), **Connect** (trusted, disconnected), **Disconnect** + **Manage** (connected) |
| Settings → **Gateway** | SoftAP SSID/password, connect sheet, disconnect, forget |

Implementation:

- [`GatewayConnectSheet`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/gateway/GatewayConnectSheet.kt) — join SoftAP, `PhoneHello`, confirm TOFU on first pin
- [`GatewayViewModel`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/gateway/GatewayViewModel.kt) — wrapper over [`GatewayWifiRepository`](../wardriver-android/app/src/main/java/net/wardriver/mobile/data/GatewayWifiRepository.kt) flows
- [`GatewayScreen`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/settings/gateway/GatewayScreen.kt) — route `settings/gateway`; edit and push SoftAP creds
- [`GatewayTrustStore`](../wardriver-android/app/src/main/java/net/wardriver/mobile/wifi/GatewayTrustStore.kt) — stable `phone_id` + local trusted flag
- [`PackLinkWifiManager`](../wardriver-android/app/src/main/java/net/wardriver/mobile/wifi/PackLinkWifiManager.kt) — `WifiNetworkSpecifier` join + TCP to `192.168.71.1:4710`
- [`PackLinkTcpSession`](../wardriver-android/app/src/main/java/net/wardriver/mobile/wifi/PackLinkTcpSession.kt) — plaintext PLNK read/write

**Forget gateway** clears local trust and sends `TrustForget` when connected. Credential changes save locally and send `SoftApConfig` when connected.

### Permissions

[`WifiPermissions`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/Permissions.kt) requests `ACCESS_FINE_LOCATION` and `NEARBY_WIFI_DEVICES` (Android 13+) for SoftAP join via `WifiNetworkSpecifier`.

## Capture settings (gateway)

[`CaptureScreen`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/settings/capture/CaptureScreen.kt) (Settings → Capture):

1. Connects preferred gateway via [`GatewayWifiRepository`](../wardriver-android/app/src/main/java/net/wardriver/mobile/data/GatewayWifiRepository.kt).
2. **Start 60s WiFi probe capture** sends PackLink `CaptureCmd` (`job_id=1`) over the TCP session.
3. Progress bar and counters from `CaptureStatus` notifies (`unpackCaptureStatusJson` JNI).
4. **Download PCAP** is a placeholder until gateway spool replay or HTTP export exists.

Control-node equivalent: `POST /api/capture/start?job=wifi_probe_pcap` (pcapng on SD).

## Session service

[`WardriveSessionService`](../wardriver-android/app/src/main/java/net/wardriver/mobile/service/WardriveSessionService.kt) is the single foreground service for both gateway PackLink and wardrive:

| Mode | When | FGS types | Notification |
|------|------|-----------|--------------|
| Gateway link | SoftAP connected, wardrive idle | `dataSync` | Connected / reconnecting to gateway |
| Wardrive | Dashboard **Start wardrive** | `dataSync` + `location` (when permitted) | Observation count, pause/resume/upload actions |

Stopping wardrive while the gateway stays connected downgrades to gateway-link mode instead of killing the process.

Notification action intents (service `onStartCommand`):

- `net.wardriver.mobile.PAUSE` — pause ingest (PackLink bulk still received; rows not stored)
- `net.wardriver.mobile.RESUME` — resume ingest
- `net.wardriver.mobile.UPLOAD` — enqueue WiGLE + Wdgwars WorkManager jobs
- Gateway notification **Disconnect** — user-initiated PackLink teardown

`START_STICKY`; wardrive notification refresh ~15 s.

### Start wardrive lifecycle

Dashboard **Start wardrive** requires a plausible phone wall clock (see below). [`WardriveSessionService`](wardriver-android/app/src/main/java/net/wardriver/mobile/service/WardriveSessionService.kt) exposes [`WardriveSessionStartState`](wardriver-android/app/src/main/java/net/wardriver/mobile/service/WardriveSessionStartState.kt) (`Idle`, `Starting`, `Running`, `Failed`) on `startStateFlow`. The wardrive card binds to this flow (not the 5 s stats poll). Failed starts surface a snackbar with the reason (foreground service blocked, enrichment open failure, phone time unset, etc.).

Preflight warnings (missing location permission, notifications denied, GPS provider off) appear on the card but do not block start unless phone time is unset. Location foreground-service type is omitted when fine location is not granted; logging stays closed until permission is granted.

### Phone wardrive clock

The wardrive clock is the Android wall clock (`System.currentTimeMillis()`). [`WardriveClock`](wardriver-android/app/src/main/java/net/wardriver/mobile/ingest/WardriveClock.kt) treats time as trusted when it falls between the APK `Build.TIME` (inclusive) and ten years later (exclusive). That clock drives PackLink `TimeSync` (`phone_unix_ms`), ingest `trusted_clock`, GPS timeline `now_ms`, and the CSV gate **Phone time unset** blocker when outside the window. **Start wardrive** is hard-blocked with the same message when the wall clock is not trusted. The app does not latch UTC from NMEA, GNSS, or fused location timestamps, and does not perform NTP in-app; OS automatic date and time is the user's responsibility. Fused location remains the position source for ingest.

Dashboard CSV **Inside home zone** matches ingest: during an active session the home-zone latch (`wardrive_suppressed`) applies; when idle, a coarse last-known fix cannot show inside home without **GPS fix** OK.

### Local data reset

Settings → **Uploads** → **Delete all recorded data** wipes `observation`, `route`, `session`, `detection`, and `network_summary`, resets the upload cursor, and preserves `fleet_roster`. An active wardrive session is stopped automatically before the wipe.

## Geofence square

Drawn on OSMDroid map; bounds stored in `session` + synced to gateway `policy_sync`. [`GeofenceMonitor`](../wardriver-mobile-core/src/geofence.rs) emits leave-square events → phone notification + optional fleet pause.

## Coverage shading (Phase 2 light)

[`CoverageGrid`](../wardriver-mobile-core/src/coverage_grid.rs) compares `route` points inside the geofence square to a coarse grid and reports unvisited cells.

## Street routing (Phase 5 optional)

[`StreetCoverageRouter`](../wardriver-mobile-core/src/street_routing.rs) exposes a stub API for future OSM graph coverage inside the square.

## JNI / Rust

`wardriver-mobile-core` builds as a **cdylib** (`libwardriver_mobile_core.so`) with `--features jni`. Kotlin loads it in [`NativeCore.kt`](../wardriver-android/app/src/main/java/net/wardriver/mobile/NativeCore.kt).

### Host tests (no JNI)

```bash
cargo test -p wardriver-mobile-core --target x86_64-unknown-linux-gnu --no-default-features -j 1
```

JSON wrapper coverage: [`wardriver-mobile-core/tests/jni_enrich.rs`](../wardriver-mobile-core/tests/jni_enrich.rs), [`wardriver-mobile-core/tests/jni_bridge.rs`](../wardriver-mobile-core/tests/jni_bridge.rs).

### Android NDK build (arm64-v8a)

Prerequisites on the build host:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk   # if not already installed
```

Android NDK via SDK Manager (AGP supplies `android.ndkDirectory`).

Gradle task [`cargoBuildMobileCore`](../wardriver-android/app/build.gradle.kts) runs before `preBuild`:

```bash
cargo ndk -t arm64-v8a -o wardriver-android/app/mobile-core/prebuilt \
  build -p wardriver-mobile-core --features jni --release
```

[`mobile-core/CMakeLists.txt`](../wardriver-android/app/mobile-core/CMakeLists.txt) imports the prebuilt `.so` for AGP `externalNativeBuild`.

Manual `.so` only (from repo root):

```bash
cargo ndk -t arm64-v8a -o wardriver-android/app/mobile-core/prebuilt \
  build -p wardriver-mobile-core --features jni --release
# -> wardriver-android/app/mobile-core/prebuilt/arm64-v8a/libwardriver_mobile_core.so
```

### Upload credentials and cursor

[`UploadPreferences`](../wardriver-android/app/src/main/java/net/wardriver/mobile/upload/UploadPreferences.kt) stores credentials in `SharedPreferences` (`wardriver_upload`):

| Key | Field | Use |
|-----|-------|-----|
| `db_marker` | `dbMarker` | Last uploaded observation `_id` (WiGLE `PREF_DB_MARKER` pattern) |
| `wigle_api_name` | `wigleApiName` | WiGLE API name (HTTP Basic user) |
| `wigle_api_token` | `wigleApiToken` | WiGLE API token (HTTP Basic password) |
| `wdgwars_api_key` | `wdgwarsApiKey` | Watch Dogs Go Wars `X-API-Key` header |

Pending rows: `observation WHERE _id > db_marker AND uploaded = 0`. The notification **Upload** action calls `UploadScheduler.scheduleAll`.

### JNI entry points

| Kotlin (`NativeCore`) | Rust (`jni.rs`) | Notes |
|-----------------------|-----------------|-------|
| `revisitRadiusM()` | `revisitRadiusM` | 50 m MacTable radius |
| `gpsTimelineOpJson(json)` | `gpsTimelineOpJson` | GPS timeline append/prune/merge (legacy JSON) |
| `gpsTimelineCreate()` / `gpsTimelineDestroy(handle)` | `gpsTimelineCreate` / `gpsTimelineDestroy` | Native GPS timeline handle |
| `gpsTimelineAppend(...)` | `gpsTimelineAppend` | Append fix without per-call timeline JSON |
| `gpsTimelineToHistJson(handle)` | `gpsTimelineToHistJson` | `gps_hist` snapshot for bulk ingest only |
| `gpsTimelineIsUsable` / `IsValidFixTime` / `IsStaleReplay` | same | Stateless timeline predicates |
| `simplifyRouteArrays(lats, lons, max)` | `simplifyRouteArrays` | Map polyline Douglas-Peucker (arrays) |
| `clusterMapMarkersArrays(...)` | `clusterMapMarkersArrays` | Map clustering with member indices |
| `coverageGridArrays(...)` | `coverageGridArrays` | Coverage fraction + unvisited cells |
| `geofenceMonitorUpdateJson(json)` | `geofenceMonitorUpdateJson` | Leave-square state machine |
| `homeZoneLatchUpdateJson(json)` | `homeZoneLatchUpdateJson` | Wardrive suppression latch |
| `homeZoneContainsJson(json)` | `homeZoneContainsJson` | Point-in-circle test |
| `ingestObservationBulkJson(wire, ctx)` | `ingestObservationBulkJson` | Session-aware wifi+ble bulk |
| `ingestCellsJson(json)` | `ingestCellsJson` | Batch cell enrich per GPS fix |
| `formatWigleHeadersJson(json)` | `formatWigleHeadersJson` | WiGLE pre-header + column header |
| `snifferDisplayJson(json)` | `snifferDisplayJson` | Quiet stale, last-seen, state label |
| `packlinkMagic()` | `packlinkMagic` | `PLNK` |
| `unpackPackLinkFrameJson(bytes)` | `unpackPackLinkFrameJson` | JSON: msg_type, payload_hex |
| `enrichWifiApJson(json)` | `enrichWifiApJson` | WiFi ingest gates + dedup |
| `enrichBleApJson(json)` | `enrichBleApJson` | BLE ingest gates + dedup |
| `enrichCellJson(json)` | `enrichCellJson` | Cell tower gates + dedup |
| `enrichmentSessionOpenJson(json)` | `enrichmentSessionOpenJson` | Open empty per-run MacTable/CellTable session |
| `enrichmentSessionCloseJson(json)` | `enrichmentSessionCloseJson` | Close session |
| `unpackObservationBulkJson(bytes)` | `unpackObservationBulkJson` | Unpack bulk (wifi + ble arrays) |
| `formatWifiCsvLineJson(json)` | `formatWifiCsvLineJson` | WiGLE WiFi data line |
| `formatBleCsvLineJson(json)` | `formatBleCsvLineJson` | WiGLE BLE data line |
| `formatCellCsvLineJson(json)` | `formatCellCsvLineJson` | WiGLE cell data line |
| `packCaptureCmdJson(json)` | `packCaptureCmdJson` | CaptureCmd wire bytes |
| `unpackCaptureStatusJson(bytes)` | `unpackCaptureStatusJson` | Capture progress JSON |

## Visual design

The Android app uses the same PACK / wolfPACK palette as the control SoftAP UI and PACK frontend:

| Source | Path |
|--------|------|
| Control web tokens | [`wardriver-control/assets/web_theme.css`](../wardriver-control/assets/web_theme.css) |
| PACK frontend | [`pack/frontend/src/index.css`](/repos/pack/frontend/src/index.css) (sibling repo) |

Compose theme lives under [`wardriver-android/app/src/main/java/net/wardriver/mobile/ui/theme/`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/theme/):

| File | Role |
|------|------|
| `PackColor.kt` | Color constants (`#0a0612` background, cyan/magenta accents) |
| `PackTypography.kt` | Orbitron headlines + Space Grotesk body (Google Fonts) |
| `PackTheme.kt` | `MaterialTheme` dark color scheme |
| `PackShapes.kt` | 10 dp card radius |

Shared UI chrome: [`ui/components/`](../wardriver-android/app/src/main/java/net/wardriver/mobile/ui/components/) (`PackGlanceCard`, `PackCard`, `PackPrimaryButton`, `PackScreenBackground`, etc.).

## Verification

Compare **observation row count** (not unique MAC count) against legacy control on the same route. Host test: [`wardriver-mobile-core/tests/e2e_observation_count.rs`](../wardriver-mobile-core/tests/e2e_observation_count.rs).
