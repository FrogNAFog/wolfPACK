# Probe request logging

Probe request logging captures non-wildcard WiFi probe requests from client devices. A probe request with a specific SSID reveals that the named network exists — even if the network is hidden (non-broadcasting SSID). This enables hidden network discovery and surveillance device detection.

Probe observations are a distinct data stream from beacon-based AP observations. They identify a **client** device searching for a network, not an AP broadcasting. They are never uploaded to WiGLE or WDGWARS and are stored locally on the phone.

## Architecture

```
Sniffer                          Gateway                          Phone
  |                                 |                                 |
  |  ESP-NOW ProbeObservationFrag  |                                 |
  |------------------------------>|                                 |
  |                                 |  PackLink ProbeObservationBulk  |
  |                                 |------------------------------>|
  |                                 |         (MsgType = 20)         |
  |                                 |                                 |
  |                                 |  GPS enrichment (phone GPS)     |
  |                                 |  Room ProbeObservationEntity    |
  |                                 |  Map purple markers             |
```

1. **Sniffer** captures non-wildcard probe requests during WiFi promiscuous mode and sends them as encrypted ESP-NOW `ProbeObservationFrag` frames (FrameType 26) to the gateway.
2. **Gateway** receives the frames, performs fragment reassembly, and forwards the complete batch as `ProbeObservationBulk` (MsgType 20) via PackLink to the phone.
3. **Phone** enriches each observation with GPS coordinates (time-anchored fix matching), deduplicates against a separate `ProbeMacTable`, and stores results in a dedicated SQLite table (`probe_observations`).
4. **Map** displays probe observations as purple markers (0xFF9B59B6) with an independent filter toggle.

## Sniffer capture

The sniffer parses probe requests in `ingest_promisc_record()` (`wardriver-sniffer/src/wifi_promisc.rs`). Wildcard probes (empty SSID) continue through the existing `ProbeReqBurst` pipeline. Non-wildcard probes (SSID present) are additionally captured as `ProbeReqWireEntry` structs:

```
src_mac(6) | ssid_len(1) | ssid(32) | channel(1) | rssi(1) | obs_mono_ms(4)
```

Records are collected during the WiFi promiscuous phase of each sweep. Probes arriving during the BLE scan window (when WiFi is paused for coexistence) are not captured — this is an inherent hardware constraint.

### Sniffer-side dedup

A `HashMap<([u8;6], [u8;32], u8), ProbeLogEntry>` keys on `(src_mac, ssid_bytes, ssid_len)` and enforces:

- **10-second cooldown** between entries for the same (MAC, SSID) pair.
- **10-entry cap** per (MAC, SSID) pair total (`PROBE_LOG_MAX_PER_ENTRY = 10`).
- **512-entry global cap** with LRU eviction by oldest `last_seen_ms` (`PROBE_DEDUP_MAX_ENTRIES = 512`).

Memory: 512 entries × ~90 bytes ≈ 46 KiB.

## Wire formats

### ProbeObservationWire

41 bytes per entry (`PROBE_OBSERVATION_WIRE_SIZE`):

| Offset | Size | Field    |
|--------|------|----------|
| 0      | 6    | src_mac  |
| 6      | 1    | ssid_len |
| 7      | 32   | ssid     |
| 39     | 1    | channel  |
| 40     | 1    | rssi     |

SSID is UTF-8, zero-padded to 32 bytes. `ssid_len` indicates the actual length (1–32).

### ProbeObservationFragV1Header

28 bytes header + 1 byte `entry_count` prefix = 29 bytes framing:

| Offset | Size | Field                   |
|--------|------|-------------------------|
| 0      | 1    | flags                   |
| 1      | 1    | frag_idx                |
| 2      | 1    | frag_total              |
| 3      | 1    | entry_count             |
| 4      | 4    | sniffer_id              |
| 8      | 4    | seq                     |
| 12     | 4    | scan_batch_id           |
| 16     | 4    | sniffer_boot_id         |
| 20     | 4    | sniffer_scan_end_ms     |
| 24     | 8    | sniffer_scan_end_unix_ms|

Each wire entry (41 bytes) follows the 29-byte header. ESP-NOW payload max is ~250 bytes, so a single frame carries at most ~4 entries (29 + 4×41 = 193 bytes). Batches larger than 4 entries are fragmented using `frag_idx`/`frag_total`, same as `ScanFragV1Header`.

### ProbeObservationBulkV1

Gateway→phone PackLink message (MsgType 20):

| Offset | Size | Field                |
|--------|------|----------------------|
| 0      | 4    | batch_id             |
| 4      | 4    | sniffer_id           |
| 8      | 8    | scan_end_unix_ms     |
| 16     | 4    | scan_end_ms          |
| 20     | 2    | count                |
| 22     | N    | payload              |

`payload` is concatenated `ProbeObservationWire` entries (41 bytes each). `count` indicates the number of entries.

## Gateway processing

The gateway ESP-NOW bridge (`wardriver-gateway/src/espnow_bridge.rs`) handles `ProbeObservationFrag` frames in `handle_probe_observation_frag()`:

1. Unpack the frame via `unpack_probe_observation_frag()`.
2. Run liveness and boot-ID checks against the fleet roster.
3. Reassemble fragments using `ingest_probe_observation_frag()` (`wardriver-core/src/scan_reasm.rs`) — keyed on `(sniffer_id, seq)` with the same dedup/fragment-tracking pattern as `ingest_wifi_scan_frag()`.
4. Pack the complete entry list into `ProbeObservationBulkV1` and forward via PackLink.
5. Note the fragment for peer-pool state and post-dump flush.

Failed unpacks increment `INGRESS_UNPACK_FAIL_PROBE` (`wardriver-stats/src/counters.rs`).

Reassembly state is maintained in `GatewayIngestState` (`wardriver-gateway/src/ingest_state.rs`):
- `reasm_probe_dedup: IndexMap<DedupKey, u64>` — dedup map keyed on `(sniffer_id, seq)`.
- `reasm_probe_state: HashMap<ReasmKey, ReasmProbe>` — partial fragment storage.

## Phone enrichment

### ProbeMacTable dedup

A dedicated `ProbeMacTable` (2048-entry `HashMap<([u8;6], String), MacEntry>`) in `wardriver-mobile-core/src/enrichment.rs` deduplicates probe observations using a composite key `(src_mac, ssid)`. This is separate from the `MacTable` used for beacon/AP observations.

Each entry tracks the last logged latitude/longitude and applies a 50-meter revisit radius (`REVISIT_RADIUS_M`), the same distance threshold as WiFi and BLE observations.

### Enrichment function

`enrich_probe_observation()` (`wardriver-mobile-core/src/enrichment.rs`) processes each wire entry:

1. Checks the enqueue gate (same as WiFi/BLE).
2. Converts SSID bytes to a string via `from_utf8_lossy` (handles non-UTF-8 SSIDs).
3. Anchors the observation timestamp against the phone's GPS timeline (`gps_anchor::observed_unix_ms_from_anchor`).
4. Matches the nearest GPS fix from `state.gps_hist` (within `MAX_FIX_TIMELINE_DELTA_MS`).
5. Runs `ProbeMacTable::allow()` to decide whether to store.
6. Returns `EnrichOutcome::StoredProbe(ProbeObservation)` or `EnrichOutcome::Dropped(reason)`.

### ProbeObservation struct

```rust
pub struct ProbeObservation {
    pub id: i64,
    pub session_id: i64,
    pub sniffer_id: u32,
    pub observed_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub src_mac: [u8; 6],   // Client MAC that sent the probe
    pub ssid: String,        // SSID the client was probing for
    pub channel: u8,
    pub rssi: i8,
}
```

`SignalKind` includes `ProbeReq` with string representation `"PROBE_REQ"`.

## Android storage

### ProbeObservationEntity

Room database entity in `probe_observations` table (migration 12→13):

```
_id             INTEGER PRIMARY KEY AUTOINCREMENT
session_id      INTEGER NOT NULL
sniffer_id      INTEGER NOT NULL
observed_unix_ms INTEGER NOT NULL
lat             REAL NOT NULL
lon             REAL NOT NULL
altitude_m      INTEGER (nullable)
accuracy_m      REAL (nullable)
src_mac         TEXT NOT NULL
ssid            TEXT NOT NULL
channel         INTEGER NOT NULL
rssi            INTEGER NOT NULL
```

Indexes on `session_id` and `src_mac`.

### ProbeObservationDao

Provides `insert()`, `insertAll()`, `countForSession()`, `getBySession()`, `countAll()`, and `deleteAll()`.

### Map display

- **Color:** Purple (`MapProbe = Color(0xFF9B59B6)`) in `PackColor`.
- **Filter:** Independent toggle in `MapLayerFilters.probe`.
- **Markers:** Probe observations are converted to `ApMarker` instances with `signalKind = "PROBE_REQ"`. The `bssid` field holds the client MAC (functionally valid for clustering; semantically distinct from an AP BSSID).
- **Viewport query:** Session-scoped, same pattern as `ObservationEntity` viewport queries.

### JNI bridge

`probe_observation_bulk_json()` in `wardriver-mobile-core/src/jni_enrich.rs` receives JSON with session ID, header fields, and payload hex. It unpacks `ProbeObservationWire` entries (41 bytes each), runs enrichment, and returns stored observations as JSON.

## Edge cases

- **SSID encoding:** Raw SSID bytes may not be valid UTF-8. `from_utf8_lossy` replaces invalid sequences with U+FFFD.
- **MAC randomization:** iOS and Android randomize MAC addresses. A single device may appear with multiple MAC addresses over time. Each (MAC, SSID) pair is a valid observation.
- **High probe volume:** Active roaming devices may probe 5–10 times per second across channels. The 10-second cooldown and 10-row cap per (MAC, SSID) prevent log flooding while preserving sufficient data points.
- **BLE scan window:** Probes during the BLE phase are lost — inherent to ESP32-C5 coexistence policy.
- **No roaming clients:** If no clients probe, the pipeline receives zero entries. This is expected behavior.

## Code map

| Component | File |
|-----------|------|
| Sniffer capture & dedup | `wardriver-sniffer/src/wifi_promisc.rs` |
| Sniffer outbound dispatch | `wardriver-sniffer/src/sniffer_espnow.rs` |
| Sniffer outbound bulk store | `wardriver-sniffer/src/sniffer_outbound_bulk.rs` |
| Probe observation wire struct | `wardriver-proto/src/lib.rs` |
| FrameType::ProbeObservationFrag (26) | `wardriver-proto/src/lib.rs` |
| PackLink ProbeObservationBulk (20) | `wardriver-packlink/src/lib.rs` |
| Fragment reassembly | `wardriver-core/src/scan_reasm.rs` |
| Gateway ESP-NOW bridge handler | `wardriver-gateway/src/espnow_bridge.rs` |
| Gateway ingest state | `wardriver-gateway/src/ingest_state.rs` |
| Unpack-fail stat counter | `wardriver-stats/src/counters.rs` |
| ProbeObservation Rust struct | `wardriver-mobile-core/src/observation.rs` |
| ProbeMacTable & enrichment | `wardriver-mobile-core/src/enrichment.rs` |
| JNI probe bulk handler | `wardriver-mobile-core/src/jni_enrich.rs` |
| JNI registration | `wardriver-mobile-core/src/jni.rs` |
| ProbeObservationEntity (Room) | `wardriver-android/.../db/WardriverDatabase.kt` |
| ProbeObservationDao | `wardriver-android/.../db/WardriverDatabase.kt` |
| Migration 12→13 | `wardriver-android/.../db/WardriverDatabase.kt` |
| Probe ingestion pipeline | `wardriver-android/.../ingest/ObservationIngestor.kt` |
| Probe map filter | `wardriver-android/.../ui/map/MapFilters.kt` |
| Probe map filter chip | `wardriver-android/.../ui/map/MapFilterBar.kt` |
| Probe marker color | `wardriver-android/.../ui/theme/PackColor.kt` |
| Probe map markers | `wardriver-android/.../ui/map/MapMarkerFactory.kt` |
| Probe ViewModel loading | `wardriver-android/.../ui/map/MapViewModel.kt` |
