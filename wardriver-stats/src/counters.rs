//! Global runtime statistics for the web UI and logs.
//!
//! Keep these cheap to update from the hot path (scan loop).

#[cfg(not(target_os = "espidf"))]
use crate::host_stub::esp_idf_sys;

use crate::alerts::{detection::*, gps::*, sniffer_slots::*, ssid_watch::*};
use crate::snapshot::*;
use crate::tier_a_build::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

#[cfg(feature = "control")]
use crate::alerts::alert_kinds::{AlertKind, ALERT_SOUND_MASK_ALL};
#[cfg(feature = "control")]
use crate::alerts::upload_progress::{UploadProgress, UploadProgressKind};

static BOOT_MS: Mutex<u64> = Mutex::new(0);
pub static MODE_REQUEST: AtomicU8 = AtomicU8::new(0);

pub const MODE_REQ_NONE: u8 = 0;
pub const MODE_REQ_MANAGEMENT: u8 = 1;
pub const MODE_REQ_WARDRIVE: u8 = 2;

/// When true, wardriving scan starts are suppressed (recon capture owns the fleet WiFi sniffers).
pub static RECON_HOLD: AtomicBool = AtomicBool::new(false);

pub static RECON_ACTIVE: AtomicU8 = AtomicU8::new(0);
pub static RECON_SESSION_ID: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW `ReconWifiFrag` payloads accepted on control (after unpack).
pub static RECON_FRAGS_RX: AtomicU32 = AtomicU32::new(0);
/// Complete probe MPDUs written into pcapng EPBs.
pub static RECON_PKTS_WRITTEN: AtomicU32 = AtomicU32::new(0);
pub static RECON_REASM_FAIL: AtomicU32 = AtomicU32::new(0);
/// Monotonic end time + duration for the active recon capture (`(0, 0)` = idle).
/// `AtomicU64` is not available on this target; lock is taken only at session start/end and on `/api/stats`.
static RECON_TIMING_MS: Mutex<(u64, u32)> = Mutex::new((0, 0));
/// Sniffer: recon fragments transmitted (control build leaves at 0).
#[allow(dead_code)]
pub static RECON_FRAGS_TX_SNIFFER: AtomicU32 = AtomicU32::new(0);

// -------- DFS diagnostics (sniffer channel hops / ESP-NOW guard) --------
/// Channel tune failures during sweep or recon (any band).
pub static DFS_HOP_SKIP: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW send blocked by guard when attempting to transmit on DFS channel.
#[allow(dead_code)]
pub static DFS_ESPNOW_BLOCKED: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "control")]
/// Per-event buzzer enable bitmask ([`AlertKind`] index = bit).
pub static ALERT_SOUND_MASK: AtomicU8 = AtomicU8::new(ALERT_SOUND_MASK_ALL);

#[cfg(feature = "control")]
pub fn set_alert_sound_mask(mask: u8) {
    ALERT_SOUND_MASK.store(mask & ALERT_SOUND_MASK_ALL, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn alert_sound_mask() -> u8 {
    ALERT_SOUND_MASK.load(Ordering::Acquire)
}

#[cfg(feature = "control")]
pub fn alert_sound_enabled(kind: AlertKind) -> bool {
    if !kind.is_user_configurable() {
        return true;
    }
    (alert_sound_mask() & kind.sound_bit()) != 0
}

#[cfg(feature = "control")]
static ALERT_BUZZER_VOLUME_PERCENT: AtomicU8 = AtomicU8::new(85);

#[cfg(feature = "control")]
pub fn set_alert_buzzer_volume_percent(percent: u8) {
    ALERT_BUZZER_VOLUME_PERCENT.store(percent.min(100), Ordering::Release);
}

#[cfg(feature = "control")]
pub fn alert_buzzer_volume_percent() -> u8 {
    ALERT_BUZZER_VOLUME_PERCENT.load(Ordering::Acquire)
}

/// Whether ESP-NOW is initialized (stays true after boot; used for dashboard).
pub static WARDRIVE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Fleet orchestrator phase: 0 idle, 1 wardrive, 2 management, 3 upload, 4 recon.
pub static FLEET_PHASE: AtomicU8 = AtomicU8::new(0);
pub static FLEET_PHASE_TRANSITIONS: AtomicU32 = AtomicU32::new(0);
static FLEET_PHASE_ENTERED_MS: Mutex<u64> = Mutex::new(0);

/// User requested fleet management: sniffers stopped, ESP-NOW stays up.
pub static FLEET_MANAGEMENT_HOLD: AtomicBool = AtomicBool::new(false);

/// GPS privacy zone: last loop decided we are inside the configured radius with a valid fix.
pub static HOME_GEOFENCE_SUPPRESSING: AtomicBool = AtomicBool::new(false);

/// Home geofence active on control: sniffers enter scan-only (detection RF only, no wardrive buffer/TX).
pub static WARDRIVE_SUPPRESSED: AtomicBool = AtomicBool::new(false);

/// BLE-role sniffers: active LE scan (SCAN_REQ) when true; passive when false (default).
pub static BLE_ACTIVE_SCAN: AtomicBool = AtomicBool::new(false);

/// Sniffer WiFi/BLE capture armed (independent of GPS / home zone). Sniffer builds never set this.
pub static SCAN_ALLOWED: AtomicBool = AtomicBool::new(false);

/// WiFi/BLE CSV row path: when false, reassembled batches stop before `try_enqueue` (see
/// [`wardriver_control::run_control`]). Requires [`SCAN_ALLOWED`] plus GPS, wall clock, and
/// outside home zone. Sniffer builds never set this (stays false).
pub static CSV_ENQUEUE_ALLOWED: AtomicBool = AtomicBool::new(false);

/// Cached `Config` booleans for `/api/stats` so we do not open NVS on every poll (avoids noisy
/// `EspNvs dropped` logs at 1 Hz from the dashboard). Updated at control boot and after NVS saves.
static NVS_CONFIG_FLAGS: AtomicU8 = AtomicU8::new(0);
const NVS_CFG_WIGLE: u8 = 1;
const NVS_CFG_WDGWARS: u8 = 2;
const NVS_CFG_PROV: u8 = 4;

pub fn set_nvs_config_flags(
    wigle_configured: bool,
    wdgwars_configured: bool,
    provisioning_complete: bool,
) {
    let mut v = 0u8;
    if wigle_configured {
        v |= NVS_CFG_WIGLE;
    }
    if wdgwars_configured {
        v |= NVS_CFG_WDGWARS;
    }
    if provisioning_complete {
        v |= NVS_CFG_PROV;
    }
    NVS_CONFIG_FLAGS.store(v, Ordering::Release);
}

pub fn nvs_config_flags() -> (bool, bool, bool) {
    let v = NVS_CONFIG_FLAGS.load(Ordering::Acquire);
    (
        (v & NVS_CFG_WIGLE) != 0,
        (v & NVS_CFG_WDGWARS) != 0,
        (v & NVS_CFG_PROV) != 0,
    )
}

/// Resolved management SoftAP SSID for `/api/stats` (no password). Set at control boot from NVS.
static MANAGEMENT_AP_SSID: Mutex<String> = Mutex::new(String::new());

pub fn set_management_ap_ssid(ssid: &str) {
    if let Ok(mut g) = MANAGEMENT_AP_SSID.lock() {
        g.clear();
        g.push_str(ssid);
    }
}

pub fn management_ap_ssid_display() -> String {
    MANAGEMENT_AP_SSID
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

static WIFI_SCAN_CYCLES: Mutex<u64> = Mutex::new(0);
static WIFI_APS_SEEN_TOTAL: Mutex<u64> = Mutex::new(0);
/// WiGLE CSV rows appended to the open SD session (after session file creation).
static WIFI_ROWS_WRITTEN: Mutex<u64> = Mutex::new(0);
static WIFI_ROWS_DROPPED_NO_ACCURACY: Mutex<u64> = Mutex::new(0);
/// Nearest GPS history sample farther than [`MAX_FIX_TIMELINE_DELTA_MS`] from observation time.
static WIFI_ROWS_DROPPED_NO_GPS_MATCH: Mutex<u64> = Mutex::new(0);
/// History match found but fix lacks usable HDOP / quality.
static WIFI_ROWS_DROPPED_NO_HDOP: Mutex<u64> = Mutex::new(0);
static WIFI_ROWS_DROPPED_DEDUP: Mutex<u64> = Mutex::new(0);
static WIFI_ROWS_DROPPED_DENYLIST: Mutex<u64> = Mutex::new(0);
static WIFI_ROWS_DROPPED_ENCODE: Mutex<u64> = Mutex::new(0);
static WIFI_ROWS_DROPPED_SD: Mutex<u64> = Mutex::new(0);
/// Reassembled **WiFi** batch dropped after reassembly, before per-row processing (entire batch).
static WIFI_BATCHES_DROPPED_FLEET_HOLD: Mutex<u64> = Mutex::new(0);
static WIFI_BATCHES_DROPPED_HOME_GEOFENCE: Mutex<u64> = Mutex::new(0);
static WIFI_BATCHES_DROPPED_CSV_GATE: Mutex<u64> = Mutex::new(0);
static WIFI_BATCHES_DROPPED_WALL_CLOCK: Mutex<u64> = Mutex::new(0);
static WIFI_BATCHES_DROPPED_NO_FIX_TIMELINE: Mutex<u64> = Mutex::new(0);
static WIFI_BATCHES_DROPPED_UTC_ENCODE: Mutex<u64> = Mutex::new(0);
static BLE_BATCHES_DROPPED_FLEET_HOLD: Mutex<u64> = Mutex::new(0);
static BLE_BATCHES_DROPPED_HOME_GEOFENCE: Mutex<u64> = Mutex::new(0);
static BLE_BATCHES_DROPPED_CSV_GATE: Mutex<u64> = Mutex::new(0);
static BLE_BATCHES_DROPPED_WALL_CLOCK: Mutex<u64> = Mutex::new(0);
static BLE_BATCHES_DROPPED_NO_FIX_TIMELINE: Mutex<u64> = Mutex::new(0);
static BLE_BATCHES_DROPPED_UTC_ENCODE: Mutex<u64> = Mutex::new(0);
/// WiGLE / MacTable geo throttle: LRU evictions (capacity or emergency shrink).
static GEO_DEDUP_EVICTIONS: Mutex<u64> = Mutex::new(0);
static GEO_DEDUP_SUPPRESSED: Mutex<u64> = Mutex::new(0);
static GEO_DEDUP_ALLOWED: Mutex<u64> = Mutex::new(0);
static UNIQUE_WIFI_APS: Mutex<u64> = Mutex::new(0);
static UNIQUE_BLE_DEVICES: Mutex<u64> = Mutex::new(0);
static MAC_TABLE_ROW_EVICTIONS: Mutex<u64> = Mutex::new(0);
static MAC_TABLE_GEO_RELEASES: Mutex<u64> = Mutex::new(0);

pub static ESPNOW_RX_QUEUE_INFLIGHT: AtomicU32 = AtomicU32::new(0);
pub static ESPNOW_RX_QUEUE_HWM: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW recv callback invocations with valid src/data (before queue enqueue).
pub static ESPNOW_RX_CB_TOTAL: AtomicU32 = AtomicU32::new(0);
/// Recv callback dropped frame because RX queue sender not wired yet (`RX_TX` None).
pub static ESPNOW_RX_CB_NO_SENDER: AtomicU32 = AtomicU32::new(0);
pub static PROMISC_RX_QUEUE_INFLIGHT: AtomicU32 = AtomicU32::new(0);
pub static PROMISC_RX_QUEUE_HWM: AtomicU32 = AtomicU32::new(0);
pub static SD_WRITER_QUEUE_INFLIGHT: AtomicU32 = AtomicU32::new(0);
pub static SD_WRITER_QUEUE_HWM: AtomicU32 = AtomicU32::new(0);

pub static CONTROL_LOOP_OVERRUNS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// Cumulative microseconds spent rebuilding prebuilt web GET snapshots (control loop).
pub static WEB_SNAPSHOT_REBUILD_US: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// Last [`now_ms`] at the start of a control main-loop iteration (`AtomicU32` — ESP has no `AtomicU64`).
pub static CONTROL_MAIN_LOOP_TICK_MS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// Duration of the most recent full main-loop iteration in milliseconds.
pub static CONTROL_MAIN_LOOP_LAST_ITER_MS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// Last [`now_ms`] when `/api/stats` JSON was rebuilt successfully.
pub static CONTROL_SNAPSHOT_BUILD_MS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// Last [`now_ms`] when httpd served `GET /api/stats` (dashboard poll; `AtomicU32` ms).
pub static LAST_DASHBOARD_STATS_FETCH_MS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// No `/api/stats` poll for this long during wardrive → switch from 1 s to 5 s rebuild cadence.
pub const DASHBOARD_STATS_IDLE_MS: u64 = 10_000;
#[cfg(feature = "control")]
/// Set by httpd on idle → active wardrive poll; consumed on the next snapshot tick.
static STATS_REBUILD_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "control")]
/// Times [`web_snapshots`] `try_write` failed (`WouldBlock` or poisoned).
pub static CONTROL_SNAPSHOT_WRITE_SKIPPED: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
/// Main-loop phase tag for stall diagnosis: see [`CONTROL_LOOP_PHASE_*`].
pub static CONTROL_MAIN_LOOP_PHASE: AtomicU8 = AtomicU8::new(0);
pub static CONTROL_LOOP_FRAMES_DRAINED_LAST: AtomicU32 = AtomicU32::new(0);
pub static CONTROL_INGEST_WORKER_BATCHES: AtomicU32 = AtomicU32::new(0);
/// Cumulative ESP-NOW frames drained by the control ingest worker.
pub static CONTROL_INGEST_FRAMES_DRAINED: AtomicU32 = AtomicU32::new(0);
/// Stack bytes used for the live ingest worker thread (after fallback selection).
pub static CONTROL_INGEST_STACK_BYTES: AtomicU32 = AtomicU32::new(0);
pub static CONTROL_STAGE_INGRESS: AtomicU32 = AtomicU32::new(0);
pub static CONTROL_STAGE_DEDUP: AtomicU32 = AtomicU32::new(0);
pub static CONTROL_STAGE_REASM: AtomicU32 = AtomicU32::new(0);
pub static CONTROL_STAGE_ROW_ENCODE: AtomicU32 = AtomicU32::new(0);
/// `unpack_*` failed after counting ingress (ScanFrag / BleScanFrag).
pub static INGRESS_UNPACK_FAIL_WIFI: AtomicU32 = AtomicU32::new(0);
pub static INGRESS_UNPACK_FAIL_BLE: AtomicU32 = AtomicU32::new(0);
/// `unpack_probe_observation_frag` failed (ProbeObservationFrag).
pub static INGRESS_UNPACK_FAIL_PROBE: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW ingress dropped: peer `(sniffer_id, mac)` not on roster (non-hello frames).
pub static INGRESS_ROSTER_DROP: AtomicU32 = AtomicU32::new(0);
/// Sniffer: RX frame from MAC other than NVS `bound_control_mac` while fleet-bound.
pub static SNIFFER_FOREIGN_FRAME_DROP: AtomicU32 = AtomicU32::new(0);
/// `cmd_seq` from the most recent broadcast [`ControlCmd::Ping`].
pub static LAST_CONTROL_PING_CMD_SEQ: AtomicU32 = AtomicU32::new(0);

/// Record the sequence number of the ping just broadcast on control.
pub fn set_last_control_ping_cmd_seq(v: u32) {
    LAST_CONTROL_PING_CMD_SEQ.store(v, Ordering::Release);
}

pub static CONTROL_DENYLIST_LOCK_SLOW: AtomicU32 = AtomicU32::new(0);
pub static REASM_OVERFLOW_EVICTIONS: AtomicU32 = AtomicU32::new(0);
pub static DEDUP_WIFI_SIZE: AtomicU32 = AtomicU32::new(0);
pub static DEDUP_BLE_SIZE: AtomicU32 = AtomicU32::new(0);
pub static MAC_TABLE_ENTRIES: AtomicU32 = AtomicU32::new(0);
pub static MAC_TABLE_GEO_ACTIVE: AtomicU32 = AtomicU32::new(0);
/// Policy cap from [`wardriver_shared::memory_policy`] at boot (0 until set).
pub static MAC_TABLE_MAX_ENTRIES: AtomicU32 = AtomicU32::new(0);
pub static MAC_TABLE_GEO_SLOTS_MAX: AtomicU32 = AtomicU32::new(0);
pub static REASM_WIFI_SIZE: AtomicU32 = AtomicU32::new(0);
pub static REASM_BLE_SIZE: AtomicU32 = AtomicU32::new(0);
pub static DEDUP_PROBE_SIZE: AtomicU32 = AtomicU32::new(0);
pub static REASM_PROBE_SIZE: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "control")]
/// Monotonic GPS time piggyback sequence for control commands (starts at 1).
pub static TIME_SET_SEQ: AtomicU32 = AtomicU32::new(1);

#[cfg(feature = "control")]
#[inline]
pub fn time_set_seq_for_wire() -> u32 {
    TIME_SET_SEQ.load(Ordering::Relaxed)
}

/// Sniffer: sweep cycles where wardrive bulk was captured but not delivered (always `0` on control-only builds).
pub static SNIFFER_BULK_LOST_CYCLES: AtomicU32 = AtomicU32::new(0);
/// Sniffer: bulk wardrive TX skipped while control home-zone suppression is active (always `0` on control-only builds).
pub static SNIFFER_WARDRIVE_SUPPRESSED_SKIPS: AtomicU32 = AtomicU32::new(0);
/// Sniffer firmware: watchdog invoked ESP-NOW `reinit_stack` recovery (always `0` on control-only builds).
pub static SNIFFER_WATCHDOG_RECOVER: AtomicU32 = AtomicU32::new(0);
/// Sniffer firmware: unified link-dead recovery at [`wardriver_core::liveness::SNIFFER_UPLINK_LIVENESS_MS`].
pub static SNIFFER_LINK_DEAD: AtomicU32 = AtomicU32::new(0);
/// Sniffer firmware: watchdog triggered `esp_restart()` after sustained loss of control RX (always `0` on control-only builds).
pub static SNIFFER_WATCHDOG_REBOOT: AtomicU32 = AtomicU32::new(0);
pub static MIN_FREE_INTERNAL_HEAP_BYTES: AtomicU32 = AtomicU32::new(u32::MAX);
pub static MIN_FREE_SPIRAM_HEAP_BYTES: AtomicU32 = AtomicU32::new(u32::MAX);

pub static SD_MOUNT_OK: AtomicBool = AtomicBool::new(false);
/// Boot mount attempts used (1..=3) when mount succeeded; max attempts when all failed.
pub static SD_MOUNT_ATTEMPTS: AtomicU32 = AtomicU32::new(0);
/// 1 when mount succeeded on attempt > 1 (soft-reset recovery).
pub static SD_MOUNT_RECOVERED: AtomicU32 = AtomicU32::new(0);
pub static SD_SESSION_CREATED: AtomicBool = AtomicBool::new(false);

static SD_HEALTH_WRITER_LAST_OK_MS: Mutex<u64> = Mutex::new(0);
static SD_HEALTH_MAIN_LAST_OK_MS: Mutex<u64> = Mutex::new(0);
/// Probe or metadata failures since boot.
pub static SD_HEALTH_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
static SD_HEALTH_LAST_FAIL_MS: Mutex<u64> = Mutex::new(0);
static SD_HEALTH_LAST_ALERT_MS: Mutex<u64> = Mutex::new(0);

/// SD diag log (`wigle/wardriver-diag.log`) observability.
///
/// These counters help determine whether SD logging is working even when UART logs
/// aren’t available (e.g., dashboard-only observation).
pub static DIAG_LOG_LINES_OK: AtomicU32 = AtomicU32::new(0);
pub static DIAG_LOG_LINES_ERR: AtomicU32 = AtomicU32::new(0);

/// When false, [`wardriver_control::storage::append_diag_log_line`] skips writes (preference from SD).
pub static DIAG_LOG_ENABLED: AtomicBool = AtomicBool::new(true);
static DIAG_LOG_LAST_ERR_MS: Mutex<u64> = Mutex::new(0);

#[cfg(feature = "control")]
/// WiGLE `pending/` capture files awaiting upload (refreshed ~60s when SD is mounted).
pub static UPLOAD_PENDING_CSV_COUNT: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "control")]
pub fn set_upload_pending_csv_count(n: u32) {
    UPLOAD_PENDING_CSV_COUNT.store(n, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn upload_pending_csv_count() -> u32 {
    UPLOAD_PENDING_CSV_COUNT.load(Ordering::Acquire)
}

#[cfg(feature = "control")]
pub const CONTROL_LOOP_PHASE_PRELUDE: u8 = 1;
#[cfg(feature = "control")]
pub const CONTROL_LOOP_PHASE_CORE: u8 = 2;
#[cfg(feature = "control")]
pub const CONTROL_LOOP_PHASE_TAIL: u8 = 3;

#[cfg(feature = "control")]
pub fn set_control_main_loop_tick_ms(now_ms: u64) {
    CONTROL_MAIN_LOOP_TICK_MS.store(now_ms.min(u64::from(u32::MAX)) as u32, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn control_main_loop_tick_ms() -> u32 {
    CONTROL_MAIN_LOOP_TICK_MS.load(Ordering::Acquire)
}

#[cfg(feature = "control")]
pub fn set_control_main_loop_last_iter_ms(ms: u64) {
    CONTROL_MAIN_LOOP_LAST_ITER_MS.store(ms.min(u64::from(u32::MAX)) as u32, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn control_main_loop_last_iter_ms() -> u32 {
    CONTROL_MAIN_LOOP_LAST_ITER_MS.load(Ordering::Acquire)
}

#[cfg(feature = "control")]
pub fn set_control_main_loop_phase(phase: u8) {
    CONTROL_MAIN_LOOP_PHASE.store(phase, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn control_main_loop_phase() -> u8 {
    CONTROL_MAIN_LOOP_PHASE.load(Ordering::Acquire)
}

#[cfg(feature = "control")]
pub fn note_control_snapshot_build_ms(now_ms: u64) {
    let t = now_ms.min(u64::from(u32::MAX)) as u32;
    CONTROL_SNAPSHOT_BUILD_MS.store(t, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn control_snapshot_build_ms() -> u32 {
    CONTROL_SNAPSHOT_BUILD_MS.load(Ordering::Acquire)
}

#[cfg(feature = "control")]
/// Ask the control main loop to rebuild `/api/stats` on its next snapshot tick.
pub fn request_stats_rebuild() {
    STATS_REBUILD_REQUESTED.store(true, Ordering::Release);
}

#[cfg(feature = "control")]
/// Returns true once per resume request; only the main loop should call this.
pub fn take_stats_rebuild_requested() -> bool {
    STATS_REBUILD_REQUESTED.swap(false, Ordering::AcqRel)
}

#[cfg(feature = "control")]
pub fn note_dashboard_stats_fetch(now_ms: u64) {
    // Read last poll time before overwriting — used to detect idle → active transitions.
    let last_poll_ms = last_dashboard_stats_fetch_ms();
    if scan_allowed() {
        let idle_gap =
            last_poll_ms != 0 && now_ms.saturating_sub(last_poll_ms) >= DASHBOARD_STATS_IDLE_MS;
        // First wardrive poll after boot may carry a snapshot built before `scan_allowed`.
        let first_poll_stale =
            last_poll_ms == 0 && control_snapshot_age_ms(now_ms) >= stats_rebuild_interval_ms();
        if idle_gap || first_poll_stale {
            request_stats_rebuild();
        }
    }
    LAST_DASHBOARD_STATS_FETCH_MS.store(now_ms.min(u64::from(u32::MAX)) as u32, Ordering::Release);
}

#[cfg(feature = "control")]
/// Active-dashboard rebuild cadence; must match [`stats_rebuild_interval_ms`] in web snapshots.
pub fn stats_rebuild_interval_ms() -> u64 {
    1_000
}

#[cfg(feature = "control")]
/// Wardrive rebuild cadence when no dashboard client polled within [`DASHBOARD_STATS_IDLE_MS`].
pub fn stats_rebuild_interval_idle_ms() -> u64 {
    5_000
}

#[cfg(feature = "control")]
pub fn last_dashboard_stats_fetch_ms() -> u64 {
    u64::from(LAST_DASHBOARD_STATS_FETCH_MS.load(Ordering::Acquire))
}

#[cfg(feature = "control")]
pub fn note_control_snapshot_write_skipped() {
    CONTROL_SNAPSHOT_WRITE_SKIPPED.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "control")]
pub fn control_snapshot_write_skipped() -> u32 {
    CONTROL_SNAPSHOT_WRITE_SKIPPED.load(Ordering::Relaxed)
}

#[cfg(feature = "control")]
pub fn control_snapshot_age_ms(now_ms: u64) -> u64 {
    let built = u64::from(control_snapshot_build_ms());
    if built == 0 {
        return 0;
    }
    now_ms.saturating_sub(built)
}

#[cfg(feature = "control")]
pub static SD_IO_RPC_TIMEOUTS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
pub static SD_IO_LAST_RPC_MS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "control")]
pub static RECON_SD_CHUNK_DROPS: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "control")]
pub fn note_sd_io_rpc_timeout() {
    SD_IO_RPC_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "control")]
pub fn note_recon_sd_chunk_drop() {
    RECON_SD_CHUNK_DROPS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "control")]
pub fn sd_io_rpc_timeouts() -> u32 {
    SD_IO_RPC_TIMEOUTS.load(Ordering::Relaxed)
}

#[cfg(feature = "control")]
pub fn sd_io_last_rpc_ms() -> u32 {
    SD_IO_LAST_RPC_MS.load(Ordering::Relaxed)
}

#[cfg(feature = "control")]
pub fn recon_sd_chunk_drops() -> u32 {
    RECON_SD_CHUNK_DROPS.load(Ordering::Relaxed)
}

/// Last uploader run: per-destination success/fail counts (upload attempts, not file rows).
pub static UPLOAD_LAST_WIGLE_OK: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_LAST_WIGLE_FAILED: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_LAST_WDGWARS_OK: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_LAST_WDGWARS_FAILED: AtomicU32 = AtomicU32::new(0);
/// `POST /uploads/run` sets this; control main loop runs STA+upload+disconnect when true.
pub static UPLOAD_REQUEST: AtomicBool = AtomicBool::new(false);
static UPLOAD_RUNS: Mutex<u64> = Mutex::new(0);

/// OLED / dashboard upload lifecycle (`upload_phase` in `/api/stats`).
pub const UPLOAD_PHASE_IDLE: u8 = 0;
pub const UPLOAD_PHASE_CONNECTING: u8 = 1;
pub const UPLOAD_PHASE_RUNNING: u8 = 2;
pub const UPLOAD_PHASE_DONE: u8 = 3;
pub const UPLOAD_PHASE_FAILED: u8 = 4;
pub static UPLOAD_PHASE: AtomicU8 = AtomicU8::new(UPLOAD_PHASE_IDLE);
/// Files with at least one pending dest attempt in the current run (1-based index while uploading).
pub static UPLOAD_FILES_DONE: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_FILES_TOTAL: AtomicU32 = AtomicU32::new(0);
/// Running per-destination attempt counts during the active upload (reset each run).
pub static UPLOAD_RUN_WIGLE_OK: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_RUN_WIGLE_FAILED: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_RUN_WDGWARS_OK: AtomicU32 = AtomicU32::new(0);
pub static UPLOAD_RUN_WDGWARS_FAILED: AtomicU32 = AtomicU32::new(0);
/// Last HTTP body transfer percent (0-100); 255 = not streaming a file body.
pub static UPLOAD_BYTE_PCT: AtomicU8 = AtomicU8::new(255);
/// OLED upload screen held until this monotonic ms (0 = no hold).
/// `AtomicU64` is not available on ESP32; hold is set once per run end.
static UPLOAD_OLED_HOLD_UNTIL_MS: Mutex<u64> = Mutex::new(0);

/// ESP-NOW receive path: frame dropped because the bounded queue was full (see `espnow::RX_QUEUE_CAPACITY`).
/// `AtomicU32` only: ESP32 `std` has no `AtomicU64`.
pub static ESPNOW_RX_DROPPED_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW receive path: `RX_TX` mutex was contended; frame dropped.
pub static ESPNOW_RX_DROPPED_MUTEX_BUSY: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW receive path: queue disconnected (shutdown).
pub static ESPNOW_RX_DROPPED_DISCONNECTED: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW receive path: incoming frame exceeded [`wardriver_shared::espnow::MAX_FRAME_LEN`].
pub static ESPNOW_RX_DROPPED_OVERSIZED: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW `esp_now_send` returned non-zero (all roles).
pub static ESPNOW_TX_FAILED: AtomicU32 = AtomicU32::new(0);
/// Sniffer: ACK send failed (subset of [`ESPNOW_TX_FAILED`]).
pub static SNIFFER_ACK_TX_FAILED: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW peer already existed and we attempted to modify it (channel/ifidx fixup).
pub static ESPNOW_PEER_MOD_ATTEMPTS: AtomicU32 = AtomicU32::new(0);
/// ESP-NOW peer modify attempt failed (non-zero rc from `esp_now_mod_peer`).
pub static ESPNOW_PEER_MOD_FAILED: AtomicU32 = AtomicU32::new(0);
/// wolfPACK peer pool: new encrypted peer slot via `ensure_peer`.
pub static PEER_POOL_JOIN: AtomicU32 = AtomicU32::new(0);
/// wolfPACK peer pool: peer removed via `evict_peer` / `evict_lru_idle`.
pub static PEER_POOL_EVICT: AtomicU32 = AtomicU32::new(0);
/// wolfPACK peer pool: stale roster `last_seen_ms` eviction (`tick_stale_liveness_evict`).
pub static PEER_POOL_STALE_LIVENESS_EVICT: AtomicU32 = AtomicU32::new(0);
/// wolfPACK peer pool: `ensure_peer` at cap invoked LRU eviction.
pub static PEER_POOL_CAP: AtomicU32 = AtomicU32::new(0);
/// wolfPACK peer pool: `mark_transport_end` entered post-dump grace (not immediate evict).
pub static PEER_POOL_TRANSPORT_END: AtomicU32 = AtomicU32::new(0);
/// wolfPACK peer pool: grace timer expired and `tick_evict_pending` removed the peer.
pub static PEER_POOL_GRACE: AtomicU32 = AtomicU32::new(0);
/// wolfPACK: `PostDumpDone` unicast sent after post-dump finish sequence.
pub static POST_DUMP_DONE_TX: AtomicU32 = AtomicU32::new(0);
/// wolfPACK: `PostDumpDone` unicast send failed.
pub static POST_DUMP_DONE_TX_FAIL: AtomicU32 = AtomicU32::new(0);
/// Post-dump RTT: `TimeSyncPing` unicast sent.
pub static TIME_SYNC_PING_TX: AtomicU32 = AtomicU32::new(0);
/// Post-dump RTT: `TimeSyncPong` received.
pub static TIME_SYNC_PONG_RX: AtomicU32 = AtomicU32::new(0);
/// Post-dump RTT: pong rejected (seq/mac mismatch or insane RTT).
pub static TIME_SYNC_RTT_REJECT: AtomicU32 = AtomicU32::new(0);
/// Post-dump RTT skipped because 10-minute interval not elapsed.
pub static TIME_SYNC_RTT_SKIPPED_NOT_DUE: AtomicU32 = AtomicU32::new(0);
/// Post-dump RTT skipped because control wall clock is not yet trusted (no GPS/SNTP).
pub static TIME_SYNC_RTT_SKIPPED_UNTRUSTED: AtomicU32 = AtomicU32::new(0);
/// wolfPACK: broadcast `PeerJoinNack` sent (cap / queue backpressure).
pub static PEER_JOIN_NACK_TX: AtomicU32 = AtomicU32::new(0);
/// wolfPACK: join-queue head expired without `PeerJoin` refresh.
pub static PEER_JOIN_QUEUE_EXPIRE: AtomicU32 = AtomicU32::new(0);
/// wolfPACK: visit-arm guard expired without bulk frag (crashed sniffer path).
pub static PEER_VISIT_ARM_TIMEOUT: AtomicU32 = AtomicU32::new(0);
/// Scan-only home-zone `PeerJoinReason::PeerRefresh` visits closed on `SnifferAck`.
pub static PEER_REFRESH_COMPLETE: AtomicU32 = AtomicU32::new(0);
/// Completed wolfPACK dump visits with telemetry recorded (`record_dump_complete`).
pub static WOLFPACK_DUMP_SAMPLES: AtomicU32 = AtomicU32::new(0);
/// Sum of per-visit queue wait (`PeerJoin` → `PeerJoinAck`); `Mutex` — ESP has no `AtomicU64`.
pub(crate) static WOLFPACK_QUEUE_MS_SUM: Mutex<u64> = Mutex::new(0);
/// Max single-visit queue wait observed since boot.
pub static WOLFPACK_QUEUE_MS_MAX: AtomicU32 = AtomicU32::new(0);
/// Sum of per-visit bulk transfer (first frag → `TRANSPORT_END`).
pub(crate) static WOLFPACK_BULK_XFER_MS_SUM: Mutex<u64> = Mutex::new(0);
/// Max single-visit bulk transfer duration observed since boot.
pub static WOLFPACK_BULK_XFER_MS_MAX: AtomicU32 = AtomicU32::new(0);

pub fn wolfpack_queue_ms_sum() -> u64 {
    WOLFPACK_QUEUE_MS_SUM.lock().map(|g| *g).unwrap_or(0)
}

pub fn wolfpack_bulk_xfer_ms_sum() -> u64 {
    WOLFPACK_BULK_XFER_MS_SUM.lock().map(|g| *g).unwrap_or(0)
}
/// Control: `broadcast_cmd` periodic `Ping` with `broadcast_only` — broadcast `esp_now_send` failed.
pub static ESPNOW_PERIODIC_PING_BROADCAST_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control: SoftAP home-channel restore invoked (see control `run`); counts restore attempts.
pub static CONTROL_WIFI_SOFTAP_RESTORE_COUNT: AtomicU32 = AtomicU32::new(0);
/// Control: successful [`wardriver_shared::espnow::refresh_stack_after_wifi`] after SoftAP restore.
pub static CONTROL_ESPNOW_STACK_REFRESH_OK: AtomicU32 = AtomicU32::new(0);
/// Control: failed ESP-NOW stack refresh after SoftAP restore.
pub static CONTROL_ESPNOW_STACK_REFRESH_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control: Hello fast-join path — `ControlWelcome` unicast send failed.
pub static HELLO_TX_WELCOME_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control: Hello fast-join path — post-welcome `StartScanning`/`StopScanning` unicast failed.
pub static HELLO_TX_CMD_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control: Hello fast-join path — BSSID denylist unicast failed.
pub static HELLO_TX_DENYLIST_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control: Hello fast-join path — optional `TimeSet` unicast failed.
pub static HELLO_TX_TIMESET_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control ingress: frame dropped before dispatch (`data[0] != PROTO_VERSION`).
pub static INGRESS_PROTO_REJECT: AtomicU32 = AtomicU32::new(0);
/// Control ingress: `SnifferHello` passed version gate (reached handler dispatch).
pub static INGRESS_HELLO_RX: AtomicU32 = AtomicU32::new(0);
/// Control ingress: `unpack_sniffer_hello` failed.
pub static INGRESS_HELLO_UNPACK_FAIL: AtomicU32 = AtomicU32::new(0);
/// Control ingress: hello registered non-roster sniffer as adoption Pending.
pub static INGRESS_HELLO_ADOPT_PENDING: AtomicU32 = AtomicU32::new(0);
/// Sniffer: broadcast `SnifferHello` ESP-NOW TX attempts (discovery + boot hello).
pub static SNIFFER_HELLO_TX: AtomicU32 = AtomicU32::new(0);
/// Promiscuous RX callback: `try_send` failed because the queue was full (sniffer builds only).
pub static PROMISC_RX_DROPPED_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);

/// Sniffer: last scan loop aborted due to promisc pressure (1) vs normal (0).
pub static SNIFFER_LAST_ABORT_PRESSURE: AtomicU8 = AtomicU8::new(0);

/// Last successful `append_row_bytes` round-trip time (µs), measured inside the SD writer
/// thread. Not directly visible on the control loop's hot path anymore (rows are queued
/// asynchronously), but surfaced on `/api/stats` so we can see when SD writes are slow.
pub static SD_APPEND_US_LAST: AtomicU32 = AtomicU32::new(0);
pub static SD_APPEND_US_MAX: AtomicU32 = AtomicU32::new(0);
/// Bounded SD writer queue overflow counter. Incremented when the control loop's `try_send`
/// returns `Full` (writer thread is behind, or SD is wedged). Treat as a flow-control signal,
/// not a correctness bug — rows dropped are counted via `add_rows_dropped_sd`.
pub static SD_WRITER_QUEUE_FULL: AtomicU32 = AtomicU32::new(0);

pub static SD_EAGER_OPEN_REQUESTED: AtomicU32 = AtomicU32::new(0);
pub static SD_EAGER_OPEN_REQUEST_DROPPED: AtomicU32 = AtomicU32::new(0);
pub static SD_EAGER_OPEN_SUCCEEDED: AtomicU32 = AtomicU32::new(0);
pub static SD_EAGER_OPEN_FAILED: AtomicU32 = AtomicU32::new(0);
pub static SD_CLEANUP_FILES_EXAMINED: AtomicU32 = AtomicU32::new(0);
pub static SD_CLEANUP_FILES_DELETED: AtomicU32 = AtomicU32::new(0);
pub static SD_CLEANUP_FILES_ERRORS: AtomicU32 = AtomicU32::new(0);

/// Operator-edited channel plan revision (control only). Increments on every saved edit;
/// the control loop watches this to re-send `SetChannelSchedule` to the fleet.
pub static CHANNEL_PLAN_REVISION: AtomicU32 = AtomicU32::new(0);
/// Count of SD load failures since boot (corrupt file, missing version, IO error).
pub static CHANNEL_PLAN_SD_LOAD_FAIL: AtomicU32 = AtomicU32::new(0);
/// Count of SD save failures since boot (IO error, SD unmounted, full filesystem).
pub static CHANNEL_PLAN_SD_SAVE_FAIL: AtomicU32 = AtomicU32::new(0);

pub fn init_boot_time() {
    if let Ok(mut g) = BOOT_MS.lock() {
        *g = now_ms();
    }
}

pub fn take_mode_request() -> u8 {
    MODE_REQUEST.swap(MODE_REQ_NONE, Ordering::AcqRel)
}

/// Returns true if manual upload was requested; clears the flag.
pub fn take_upload_request() -> bool {
    UPLOAD_REQUEST.swap(false, Ordering::AcqRel)
}

pub fn wardrive_active() -> bool {
    WARDRIVE_ACTIVE.load(Ordering::Acquire)
}

pub fn set_wardrive_active(v: bool) {
    WARDRIVE_ACTIVE.store(v, Ordering::Release);
}

pub fn fleet_phase() -> u8 {
    FLEET_PHASE.load(Ordering::Acquire)
}

pub fn fleet_phase_transitions() -> u32 {
    FLEET_PHASE_TRANSITIONS.load(Ordering::Relaxed)
}

pub fn fleet_phase_entered_ms() -> u64 {
    FLEET_PHASE_ENTERED_MS.lock().map(|g| *g).unwrap_or(0)
}

/// Publish orchestrator phase and transition metadata for `/api/stats`.
pub fn set_fleet_phase_metrics(phase: u8, transitions: u32, entered_ms: u64) {
    FLEET_PHASE.store(phase, Ordering::Release);
    FLEET_PHASE_TRANSITIONS.store(transitions, Ordering::Relaxed);
    if let Ok(mut g) = FLEET_PHASE_ENTERED_MS.lock() {
        *g = entered_ms;
    }
}

pub fn fleet_management_hold() -> bool {
    FLEET_MANAGEMENT_HOLD.load(Ordering::Acquire)
}

pub fn set_fleet_management_hold(v: bool) {
    FLEET_MANAGEMENT_HOLD.store(v, Ordering::Release);
}

#[inline]
pub fn recon_hold() -> bool {
    RECON_HOLD.load(Ordering::Acquire)
}

pub fn set_recon_hold(v: bool) {
    RECON_HOLD.store(v, Ordering::Release);
}

/// `(end_mono_ms, duration_ms)` for the active recon capture; `(0, 0)` when idle.
#[inline]
pub fn recon_timing_ms() -> (u64, u32) {
    RECON_TIMING_MS.lock().map(|g| *g).unwrap_or((0, 0))
}

pub fn set_recon_timing_ms(end_mono_ms: u64, duration_ms: u32) {
    if let Ok(mut g) = RECON_TIMING_MS.lock() {
        *g = (end_mono_ms, duration_ms);
    }
}

pub fn clear_recon_timing_ms() {
    if let Ok(mut g) = RECON_TIMING_MS.lock() {
        *g = (0, 0);
    }
}

pub fn set_home_geofence_suppressing(v: bool) {
    HOME_GEOFENCE_SUPPRESSING.store(v, Ordering::Release);
}

pub fn set_wardrive_suppressed(v: bool) {
    WARDRIVE_SUPPRESSED.store(v, Ordering::Release);
}

#[inline]
pub fn wardrive_suppressed() -> bool {
    WARDRIVE_SUPPRESSED.load(Ordering::Acquire)
}

pub fn set_ble_active_scan(v: bool) {
    BLE_ACTIVE_SCAN.store(v, Ordering::Release);
}

#[inline]
pub fn ble_active_scan() -> bool {
    BLE_ACTIVE_SCAN.load(Ordering::Acquire)
}

#[inline]
pub fn scan_allowed() -> bool {
    SCAN_ALLOWED.load(Ordering::Acquire)
}

pub fn set_scan_allowed(v: bool) {
    SCAN_ALLOWED.store(v, Ordering::Release);
}

pub fn csv_enqueue_allowed() -> bool {
    CSV_ENQUEUE_ALLOWED.load(Ordering::Acquire)
}

pub fn set_csv_enqueue_allowed(v: bool) {
    CSV_ENQUEUE_ALLOWED.store(v, Ordering::Release);
}

pub fn now_ms() -> u64 {
    // Monotonic milliseconds since boot. Do NOT use wall clock here: SNTP/GPS
    // time sync may jump the system time, which breaks dashboard uptime and
    // "last seen" timestamps.
    (unsafe { esp_idf_sys::esp_timer_get_time() } as u64) / 1000
}

/// Current free heap (bytes). IDF: sum over heaps when using default.
pub fn free_heap_bytes() -> u32 {
    // SAFETY: no arguments; callable from any task.
    unsafe { esp_idf_sys::esp_get_free_heap_size() }
}

/// Current free internal-capability heap bytes.
pub fn free_internal_heap_bytes() -> u32 {
    // SAFETY: capability query only; callable from any task.
    unsafe { esp_idf_sys::heap_caps_get_free_size(esp_idf_sys::MALLOC_CAP_INTERNAL as u32) as u32 }
}

/// Total internal DRAM heap size (bytes). Used with [`free_internal_heap_bytes`] for internal-only views.
pub fn internal_heap_total_bytes() -> u32 {
    unsafe { esp_idf_sys::heap_caps_get_total_size(esp_idf_sys::MALLOC_CAP_INTERNAL as u32) as u32 }
}

/// Total SPIRAM heap size (bytes). Zero when SPIRAM is not enabled or not initialized.
pub fn spiram_heap_total_bytes() -> u32 {
    unsafe { esp_idf_sys::heap_caps_get_total_size(esp_idf_sys::MALLOC_CAP_SPIRAM as u32) as u32 }
}

/// Saturating sum: internal + SPIRAM free bytes (usable combined heap when PSRAM is present).
pub fn free_heap_internal_plus_spiram_bytes() -> u32 {
    free_internal_heap_bytes().saturating_add(free_spiram_heap_bytes())
}

/// Saturating sum: internal + SPIRAM total heap bytes.
pub fn total_heap_internal_plus_spiram_bytes() -> u32 {
    internal_heap_total_bytes().saturating_add(spiram_heap_total_bytes())
}

/// Whole-number percent free for a `(free, total)` snapshot (0–100). [`None`] if `total == 0`.
pub fn heap_avail_pct_from(free: u32, total: u32) -> Option<u8> {
    if total == 0 {
        return None;
    }
    let free = free.min(total);
    Some(((free as u64 * 100) / total as u64).min(100) as u8)
}

/// Percent of combined internal + SPIRAM heap that is currently free (0–100).
pub fn heap_avail_pct() -> u8 {
    heap_avail_pct_from(
        free_heap_internal_plus_spiram_bytes(),
        total_heap_internal_plus_spiram_bytes(),
    )
    .unwrap_or(0)
}

/// Current free PSRAM-capability heap bytes (0 when SPIRAM is not enabled/initialized).
pub fn free_spiram_heap_bytes() -> u32 {
    // SAFETY: capability query only; callable from any task.
    unsafe { esp_idf_sys::heap_caps_get_free_size(esp_idf_sys::MALLOC_CAP_SPIRAM as u32) as u32 }
}

pub fn refresh_memory_low_watermarks() {
    let internal = free_internal_heap_bytes();
    let spiram = free_spiram_heap_bytes();
    let prev_i = MIN_FREE_INTERNAL_HEAP_BYTES.load(Ordering::Relaxed);
    if internal < prev_i {
        MIN_FREE_INTERNAL_HEAP_BYTES.store(internal, Ordering::Relaxed);
    }
    let prev_s = MIN_FREE_SPIRAM_HEAP_BYTES.load(Ordering::Relaxed);
    if spiram < prev_s {
        MIN_FREE_SPIRAM_HEAP_BYTES.store(spiram, Ordering::Relaxed);
    }
}

/// Smallest free heap (bytes) seen since boot; useful to spot OOM risk after long runs.
pub fn min_ever_free_heap_bytes() -> u32 {
    // SAFETY: no arguments; IDF maintains a low-water mark.
    unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() }
}

pub fn inc_scan_cycle() {
    if let Ok(mut g) = WIFI_SCAN_CYCLES.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn add_aps_seen(n: usize) {
    if let Ok(mut g) = WIFI_APS_SEEN_TOTAL.lock() {
        *g = g.saturating_add(n as u64);
    }
}

/// Count rows successfully written via [`wardriver_control::storage::Session::append_row_bytes`] (session must be open).
pub fn add_rows_written(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_WRITTEN.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_rows_dropped_no_gps_match(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_DROPPED_NO_GPS_MATCH.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_rows_dropped_no_hdop(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_DROPPED_NO_HDOP.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_rows_dropped_dedup(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_DROPPED_DEDUP.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_geodeduper_evictions(n: u32) {
    if n == 0 {
        return;
    }
    if let Ok(mut g) = GEO_DEDUP_EVICTIONS.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn geodeduper_evictions() -> u64 {
    GEO_DEDUP_EVICTIONS.lock().map(|g| *g).unwrap_or(0)
}

pub fn add_geodeduper_allowed(n: u32) {
    if let Ok(mut g) = GEO_DEDUP_ALLOWED.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_geodeduper_suppressed(n: u32) {
    if let Ok(mut g) = GEO_DEDUP_SUPPRESSED.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn geodeduper_allowed() -> u64 {
    GEO_DEDUP_ALLOWED.lock().map(|g| *g).unwrap_or(0)
}

pub fn geodeduper_suppressed() -> u64 {
    GEO_DEDUP_SUPPRESSED.lock().map(|g| *g).unwrap_or(0)
}

pub fn add_unique_wifi_aps(n: u32) {
    if n == 0 {
        return;
    }
    if let Ok(mut g) = UNIQUE_WIFI_APS.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn unique_wifi_aps() -> u64 {
    UNIQUE_WIFI_APS.lock().map(|g| *g).unwrap_or(0)
}

pub fn add_unique_ble_devices(n: u32) {
    if n == 0 {
        return;
    }
    if let Ok(mut g) = UNIQUE_BLE_DEVICES.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn unique_ble_devices() -> u64 {
    UNIQUE_BLE_DEVICES.lock().map(|g| *g).unwrap_or(0)
}

pub fn add_mac_table_row_evictions(n: u32) {
    if n == 0 {
        return;
    }
    if let Ok(mut g) = MAC_TABLE_ROW_EVICTIONS.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn mac_table_row_evictions() -> u64 {
    MAC_TABLE_ROW_EVICTIONS.lock().map(|g| *g).unwrap_or(0)
}

pub fn add_mac_table_geo_releases(n: u32) {
    if n == 0 {
        return;
    }
    if let Ok(mut g) = MAC_TABLE_GEO_RELEASES.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn mac_table_geo_releases() -> u64 {
    MAC_TABLE_GEO_RELEASES.lock().map(|g| *g).unwrap_or(0)
}

pub fn add_rows_dropped_denylist(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_DROPPED_DENYLIST.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_rows_dropped_encode(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_DROPPED_ENCODE.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn add_rows_dropped_sd(n: usize) {
    if let Ok(mut g) = WIFI_ROWS_DROPPED_SD.lock() {
        *g = g.saturating_add(n as u64);
    }
}

pub fn set_sd_mounted(ok: bool) {
    SD_MOUNT_OK.store(ok, Ordering::Release);
}

pub fn record_sd_mount_success(attempt: u32) {
    SD_MOUNT_ATTEMPTS.store(attempt, Ordering::Relaxed);
    if attempt > 1 {
        SD_MOUNT_RECOVERED.store(1, Ordering::Relaxed);
    }
}

pub fn record_sd_mount_failed(attempts: u32) {
    SD_MOUNT_ATTEMPTS.store(attempts, Ordering::Relaxed);
}

pub fn sd_mounted() -> bool {
    SD_MOUNT_OK.load(Ordering::Acquire)
}

pub fn set_sd_session_created(ok: bool) {
    SD_SESSION_CREATED.store(ok, Ordering::Release);
}

pub fn sd_health_writer_last_ok_ms() -> u64 {
    SD_HEALTH_WRITER_LAST_OK_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn set_sd_health_main_last_ok_ms(ms: u64) {
    if let Ok(mut g) = SD_HEALTH_MAIN_LAST_OK_MS.lock() {
        *g = ms;
    }
}

pub fn sd_health_main_last_ok_ms() -> u64 {
    SD_HEALTH_MAIN_LAST_OK_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn record_sd_health_fail() {
    SD_HEALTH_FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut g) = SD_HEALTH_LAST_FAIL_MS.lock() {
        *g = now_ms();
    }
}

pub fn sd_health_fail_count() -> u32 {
    SD_HEALTH_FAIL_COUNT.load(Ordering::Relaxed)
}

pub fn sd_health_last_fail_ms() -> u64 {
    SD_HEALTH_LAST_FAIL_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn sd_health_last_alert_ms() -> u64 {
    SD_HEALTH_LAST_ALERT_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn set_sd_health_last_alert_ms(ms: u64) {
    if let Ok(mut g) = SD_HEALTH_LAST_ALERT_MS.lock() {
        *g = ms;
    }
}

/// `true` when the card is still marked mounted (either probe path succeeded recently is left to the dashboard).
#[must_use]
pub fn sd_health_ok() -> bool {
    sd_mounted()
}

pub fn diag_log_line_ok() {
    DIAG_LOG_LINES_OK.fetch_add(1, Ordering::Relaxed);
}

pub fn diag_log_line_err() {
    DIAG_LOG_LINES_ERR.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut g) = DIAG_LOG_LAST_ERR_MS.lock() {
        *g = now_ms();
    }
}

pub fn diag_log_lines_ok() -> u32 {
    DIAG_LOG_LINES_OK.load(Ordering::Relaxed)
}

pub fn diag_log_lines_err() -> u32 {
    DIAG_LOG_LINES_ERR.load(Ordering::Relaxed)
}

pub fn diag_log_last_err_ms() -> u64 {
    DIAG_LOG_LAST_ERR_MS.lock().map(|g| *g).unwrap_or(0)
}

#[must_use]
pub fn diag_log_enabled() -> bool {
    DIAG_LOG_ENABLED.load(Ordering::Acquire)
}

pub fn set_diag_log_enabled(enabled: bool) {
    DIAG_LOG_ENABLED.store(enabled, Ordering::Release);
}

pub fn record_upload(wigle_ok: u32, wigle_fail: u32, wdg_ok: u32, wdg_fail: u32) {
    if let Ok(mut g) = UPLOAD_RUNS.lock() {
        *g = g.saturating_add(1);
    }
    UPLOAD_LAST_WIGLE_OK.store(wigle_ok, Ordering::Release);
    UPLOAD_LAST_WIGLE_FAILED.store(wigle_fail, Ordering::Release);
    UPLOAD_LAST_WDGWARS_OK.store(wdg_ok, Ordering::Release);
    UPLOAD_LAST_WDGWARS_FAILED.store(wdg_fail, Ordering::Release);
    UPLOAD_PHASE.store(UPLOAD_PHASE_DONE, Ordering::Release);
}

pub fn set_upload_phase(phase: u8) {
    UPLOAD_PHASE.store(phase, Ordering::Release);
}

pub fn upload_phase() -> u8 {
    UPLOAD_PHASE.load(Ordering::Acquire)
}

pub fn upload_phase_name(phase: u8) -> &'static str {
    match phase {
        UPLOAD_PHASE_CONNECTING => "connecting",
        UPLOAD_PHASE_RUNNING => "running",
        UPLOAD_PHASE_DONE => "done",
        UPLOAD_PHASE_FAILED => "failed",
        _ => "idle",
    }
}

pub fn upload_in_progress() -> bool {
    matches!(
        upload_phase(),
        UPLOAD_PHASE_CONNECTING | UPLOAD_PHASE_RUNNING
    )
}

pub fn reset_upload_run_counters() {
    UPLOAD_FILES_DONE.store(0, Ordering::Release);
    UPLOAD_FILES_TOTAL.store(0, Ordering::Release);
    UPLOAD_RUN_WIGLE_OK.store(0, Ordering::Release);
    UPLOAD_RUN_WIGLE_FAILED.store(0, Ordering::Release);
    UPLOAD_RUN_WDGWARS_OK.store(0, Ordering::Release);
    UPLOAD_RUN_WDGWARS_FAILED.store(0, Ordering::Release);
    UPLOAD_BYTE_PCT.store(255, Ordering::Release);
}

/// Keep the upload OLED screen visible for `duration_ms` after the run ends.
pub fn set_upload_oled_hold_ms(duration_ms: u64) {
    let until = now_ms().saturating_add(duration_ms);
    if let Ok(mut g) = UPLOAD_OLED_HOLD_UNTIL_MS.lock() {
        *g = until;
    }
}

/// True while the post-upload OLED hold window has not expired.
pub fn upload_oled_hold_active() -> bool {
    let until = UPLOAD_OLED_HOLD_UNTIL_MS.lock().map(|g| *g).unwrap_or(0);
    until > 0 && now_ms() < until
}

pub fn clear_upload_phase() {
    UPLOAD_PHASE.store(UPLOAD_PHASE_IDLE, Ordering::Release);
}

#[cfg(feature = "control")]
pub fn set_upload_progress(p: &UploadProgress) {
    let phase = match p.kind {
        UploadProgressKind::Connecting => UPLOAD_PHASE_CONNECTING,
        UploadProgressKind::Scanning
        | UploadProgressKind::Uploading { .. }
        | UploadProgressKind::NothingPending => UPLOAD_PHASE_RUNNING,
        UploadProgressKind::Done => UPLOAD_PHASE_DONE,
        UploadProgressKind::Failed => UPLOAD_PHASE_FAILED,
    };
    UPLOAD_PHASE.store(phase, Ordering::Release);
    if let UploadProgressKind::Uploading {
        file_index,
        file_total,
        byte_pct,
        ..
    } = p.kind
    {
        UPLOAD_FILES_DONE.store(file_index, Ordering::Release);
        UPLOAD_FILES_TOTAL.store(file_total, Ordering::Release);
        UPLOAD_BYTE_PCT.store(byte_pct, Ordering::Release);
    }
    UPLOAD_RUN_WIGLE_OK.store(p.wigle_ok, Ordering::Release);
    UPLOAD_RUN_WIGLE_FAILED.store(p.wigle_fail, Ordering::Release);
    UPLOAD_RUN_WDGWARS_OK.store(p.wdgwars_ok, Ordering::Release);
    UPLOAD_RUN_WDGWARS_FAILED.store(p.wdgwars_fail, Ordering::Release);
}

pub fn boot_ms() -> u64 {
    BOOT_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_scan_cycles() -> u64 {
    WIFI_SCAN_CYCLES.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_aps_seen_total() -> u64 {
    WIFI_APS_SEEN_TOTAL.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_rows_written() -> u64 {
    WIFI_ROWS_WRITTEN.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_rows_dropped_no_accuracy() -> u64 {
    WIFI_ROWS_DROPPED_NO_ACCURACY
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}

pub fn wifi_rows_dropped_no_gps_match() -> u64 {
    WIFI_ROWS_DROPPED_NO_GPS_MATCH
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}

pub fn wifi_rows_dropped_no_hdop() -> u64 {
    WIFI_ROWS_DROPPED_NO_HDOP.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_rows_dropped_dedup() -> u64 {
    WIFI_ROWS_DROPPED_DEDUP.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_rows_dropped_denylist() -> u64 {
    WIFI_ROWS_DROPPED_DENYLIST.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_rows_dropped_encode() -> u64 {
    WIFI_ROWS_DROPPED_ENCODE.lock().map(|g| *g).unwrap_or(0)
}

pub fn wifi_rows_dropped_sd() -> u64 {
    WIFI_ROWS_DROPPED_SD.lock().map(|g| *g).unwrap_or(0)
}

fn add_batch_drop(m: &Mutex<u64>) {
    if let Ok(mut g) = m.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn add_wifi_batches_dropped_fleet_hold() {
    add_batch_drop(&WIFI_BATCHES_DROPPED_FLEET_HOLD);
}
pub fn add_wifi_batches_dropped_home_geofence() {
    add_batch_drop(&WIFI_BATCHES_DROPPED_HOME_GEOFENCE);
}
pub fn add_wifi_batches_dropped_csv_gate() {
    add_batch_drop(&WIFI_BATCHES_DROPPED_CSV_GATE);
}
pub fn add_wifi_batches_dropped_wall_clock() {
    add_batch_drop(&WIFI_BATCHES_DROPPED_WALL_CLOCK);
}
pub fn add_wifi_batches_dropped_no_fix_timeline() {
    add_batch_drop(&WIFI_BATCHES_DROPPED_NO_FIX_TIMELINE);
}
pub fn add_ble_batches_dropped_fleet_hold() {
    add_batch_drop(&BLE_BATCHES_DROPPED_FLEET_HOLD);
}
pub fn add_ble_batches_dropped_home_geofence() {
    add_batch_drop(&BLE_BATCHES_DROPPED_HOME_GEOFENCE);
}
pub fn add_ble_batches_dropped_csv_gate() {
    add_batch_drop(&BLE_BATCHES_DROPPED_CSV_GATE);
}
pub fn add_ble_batches_dropped_wall_clock() {
    add_batch_drop(&BLE_BATCHES_DROPPED_WALL_CLOCK);
}
pub fn add_ble_batches_dropped_no_fix_timeline() {
    add_batch_drop(&BLE_BATCHES_DROPPED_NO_FIX_TIMELINE);
}

pub fn wifi_batches_dropped_fleet_hold() -> u64 {
    WIFI_BATCHES_DROPPED_FLEET_HOLD
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn wifi_batches_dropped_home_geofence() -> u64 {
    WIFI_BATCHES_DROPPED_HOME_GEOFENCE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn wifi_batches_dropped_csv_gate() -> u64 {
    WIFI_BATCHES_DROPPED_CSV_GATE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn wifi_batches_dropped_wall_clock() -> u64 {
    WIFI_BATCHES_DROPPED_WALL_CLOCK
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn wifi_batches_dropped_no_fix_timeline() -> u64 {
    WIFI_BATCHES_DROPPED_NO_FIX_TIMELINE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn wifi_batches_dropped_utc_encode() -> u64 {
    WIFI_BATCHES_DROPPED_UTC_ENCODE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn ble_batches_dropped_fleet_hold() -> u64 {
    BLE_BATCHES_DROPPED_FLEET_HOLD
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn ble_batches_dropped_home_geofence() -> u64 {
    BLE_BATCHES_DROPPED_HOME_GEOFENCE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn ble_batches_dropped_csv_gate() -> u64 {
    BLE_BATCHES_DROPPED_CSV_GATE.lock().map(|g| *g).unwrap_or(0)
}
pub fn ble_batches_dropped_wall_clock() -> u64 {
    BLE_BATCHES_DROPPED_WALL_CLOCK
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn ble_batches_dropped_no_fix_timeline() -> u64 {
    BLE_BATCHES_DROPPED_NO_FIX_TIMELINE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}
pub fn ble_batches_dropped_utc_encode() -> u64 {
    BLE_BATCHES_DROPPED_UTC_ENCODE
        .lock()
        .map(|g| *g)
        .unwrap_or(0)
}

pub fn upload_runs() -> u64 {
    UPLOAD_RUNS.lock().map(|g| *g).unwrap_or(0)
}

pub fn espnow_rx_dropped_queue_full() -> u32 {
    ESPNOW_RX_DROPPED_QUEUE_FULL.load(Ordering::Relaxed)
}

pub fn espnow_rx_dropped_mutex_busy() -> u32 {
    ESPNOW_RX_DROPPED_MUTEX_BUSY.load(Ordering::Relaxed)
}

pub fn espnow_rx_dropped_disconnected() -> u32 {
    ESPNOW_RX_DROPPED_DISCONNECTED.load(Ordering::Relaxed)
}

pub fn espnow_rx_dropped_oversized() -> u32 {
    ESPNOW_RX_DROPPED_OVERSIZED.load(Ordering::Relaxed)
}

pub fn espnow_tx_failed() -> u32 {
    ESPNOW_TX_FAILED.load(Ordering::Relaxed)
}

pub fn sniffer_ack_tx_failed() -> u32 {
    SNIFFER_ACK_TX_FAILED.load(Ordering::Relaxed)
}

pub fn sniffer_bulk_lost_cycles() -> u32 {
    SNIFFER_BULK_LOST_CYCLES.load(Ordering::Relaxed)
}

pub fn inc_sniffer_bulk_lost_cycle() {
    SNIFFER_BULK_LOST_CYCLES.fetch_add(1, Ordering::Relaxed);
}

pub fn time_sync_ping_tx() {
    TIME_SYNC_PING_TX.fetch_add(1, Ordering::Relaxed);
}

pub fn time_sync_pong_rx() {
    TIME_SYNC_PONG_RX.fetch_add(1, Ordering::Relaxed);
}

pub fn time_sync_rtt_reject() {
    TIME_SYNC_RTT_REJECT.fetch_add(1, Ordering::Relaxed);
}

pub fn time_sync_rtt_skipped_not_due() {
    TIME_SYNC_RTT_SKIPPED_NOT_DUE.fetch_add(1, Ordering::Relaxed);
}

pub fn time_sync_rtt_skipped_untrusted() {
    TIME_SYNC_RTT_SKIPPED_UNTRUSTED.fetch_add(1, Ordering::Relaxed);
}

pub fn espnow_peer_mod_attempts() -> u32 {
    ESPNOW_PEER_MOD_ATTEMPTS.load(Ordering::Relaxed)
}

pub fn espnow_peer_mod_failed() -> u32 {
    ESPNOW_PEER_MOD_FAILED.load(Ordering::Relaxed)
}

/// Always zero on control firmware; incremented on sniffer nodes when the promisc queue overflows.
pub fn promisc_rx_dropped_queue_full() -> u32 {
    PROMISC_RX_DROPPED_QUEUE_FULL.load(Ordering::Relaxed)
}

pub fn sniffer_last_abort_pressure() -> u8 {
    SNIFFER_LAST_ABORT_PRESSURE.load(Ordering::Relaxed)
}

/// Publish the current operator-edited plan revision (control side). Bump on every save.
pub fn set_channel_plan_revision(rev: u32) {
    CHANNEL_PLAN_REVISION.store(rev, Ordering::Release);
}

/// Increment after any SD-side channel plan load failure (logged separately).
pub fn note_channel_plan_sd_load_fail() {
    CHANNEL_PLAN_SD_LOAD_FAIL.fetch_add(1, Ordering::Relaxed);
}

/// Increment after any SD-side channel plan save failure (logged separately).
pub fn note_channel_plan_sd_save_fail() {
    CHANNEL_PLAN_SD_SAVE_FAIL.fetch_add(1, Ordering::Relaxed);
}

/// DFS telemetry helpers (sniffer builds only).
pub fn increment_dfs_hop_skip() {
    DFS_HOP_SKIP.fetch_add(1, Ordering::Relaxed);
}

pub fn increment_dfs_espnow_blocked() {
    DFS_ESPNOW_BLOCKED.fetch_add(1, Ordering::Relaxed);
}

pub fn record_sd_append_us(us: u32) {
    SD_APPEND_US_LAST.store(us, Ordering::Relaxed);
    let prev = SD_APPEND_US_MAX.load(Ordering::Relaxed);
    if us > prev {
        SD_APPEND_US_MAX.store(us, Ordering::Relaxed);
    }
}

pub fn sd_append_us_last() -> u32 {
    SD_APPEND_US_LAST.load(Ordering::Relaxed)
}

pub fn sd_append_us_max() -> u32 {
    SD_APPEND_US_MAX.load(Ordering::Relaxed)
}

pub fn sd_writer_queue_full() -> u32 {
    SD_WRITER_QUEUE_FULL.load(Ordering::Relaxed)
}

pub fn sd_eager_open_requested() {
    SD_EAGER_OPEN_REQUESTED.fetch_add(1, Ordering::Relaxed);
}

pub fn sd_eager_open_request_dropped() {
    SD_EAGER_OPEN_REQUEST_DROPPED.fetch_add(1, Ordering::Relaxed);
}

pub fn sd_eager_open_succeeded() {
    SD_EAGER_OPEN_SUCCEEDED.fetch_add(1, Ordering::Relaxed);
}

pub fn sd_eager_open_failed() {
    SD_EAGER_OPEN_FAILED.fetch_add(1, Ordering::Relaxed);
}

pub fn sd_eager_open_requested_count() -> u32 {
    SD_EAGER_OPEN_REQUESTED.load(Ordering::Relaxed)
}

pub fn sd_eager_open_request_dropped_count() -> u32 {
    SD_EAGER_OPEN_REQUEST_DROPPED.load(Ordering::Relaxed)
}

pub fn sd_eager_open_succeeded_count() -> u32 {
    SD_EAGER_OPEN_SUCCEEDED.load(Ordering::Relaxed)
}

pub fn sd_eager_open_failed_count() -> u32 {
    SD_EAGER_OPEN_FAILED.load(Ordering::Relaxed)
}

pub fn record_sd_cleanup(examined: u32, deleted: u32, errors: u32) {
    SD_CLEANUP_FILES_EXAMINED.store(examined, Ordering::Relaxed);
    SD_CLEANUP_FILES_DELETED.store(deleted, Ordering::Relaxed);
    SD_CLEANUP_FILES_ERRORS.store(errors, Ordering::Relaxed);
}

pub fn sd_cleanup_files_examined() -> u32 {
    SD_CLEANUP_FILES_EXAMINED.load(Ordering::Relaxed)
}

pub fn sd_cleanup_files_deleted() -> u32 {
    SD_CLEANUP_FILES_DELETED.load(Ordering::Relaxed)
}

pub fn sd_cleanup_files_errors() -> u32 {
    SD_CLEANUP_FILES_ERRORS.load(Ordering::Relaxed)
}

pub fn queue_inflight_inc(inflight: &AtomicU32, hwm: &AtomicU32) {
    let now = inflight.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let prev = hwm.load(Ordering::Relaxed);
    if now > prev {
        hwm.store(now, Ordering::Relaxed);
    }
}

pub fn queue_inflight_dec(inflight: &AtomicU32) {
    let mut cur = inflight.load(Ordering::Relaxed);
    while cur > 0 {
        match inflight.compare_exchange_weak(cur, cur - 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
}

/// Minimum spacing between SetRole unicasts for the same sniffer (ACK-driven resend).
pub const ROLE_SETROLE_MIN_INTERVAL_MS: u64 = 1_000;

#[derive(Clone, Copy, Debug)]
pub struct PendingRoleChange {
    pub mac: [u8; 6],
    pub desired_role: u8,
    /// `0` = never sent; main loop sends first unicast when zero.
    pub last_send_ms: u64,
}

static PENDING_ROLE_CHANGES: Mutex<BTreeMap<u32, PendingRoleChange>> = Mutex::new(BTreeMap::new());

/// Outcome of comparing a sniffer ACK role against a pending operator SetRole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingRoleReconcile {
    /// ACK role matches desired; pending entry removed.
    Clear,
    /// ACK role still wrong and resend interval elapsed.
    Resend { desired_role: u8 },
    /// No pending change, or resend not yet due.
    None,
}

pub fn enqueue_role_change(sniffer_id: u32, mac: [u8; 6], desired_role: u8) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut map) = PENDING_ROLE_CHANGES.lock() {
        map.insert(
            sniffer_id,
            PendingRoleChange {
                mac,
                desired_role: desired_role.min(1),
                last_send_ms: 0,
            },
        );
    }
}

pub fn pending_roles_needing_first_send() -> Vec<(u32, [u8; 6], u8)> {
    PENDING_ROLE_CHANGES
        .lock()
        .map(|map| {
            map.iter()
                .filter(|(_, p)| p.last_send_ms == 0)
                .map(|(&id, p)| (id, p.mac, p.desired_role))
                .collect()
        })
        .unwrap_or_default()
}

pub fn pending_role_for(sniffer_id: u32) -> Option<([u8; 6], u8)> {
    PENDING_ROLE_CHANGES
        .lock()
        .ok()
        .and_then(|map| map.get(&sniffer_id).map(|p| (p.mac, p.desired_role)))
}

pub fn mark_setrole_sent(sniffer_id: u32, now_ms: u64) {
    if let Ok(mut map) = PENDING_ROLE_CHANGES.lock() {
        if let Some(p) = map.get_mut(&sniffer_id) {
            p.last_send_ms = now_ms;
        }
    }
}

pub fn reconcile_pending_role_after_ack(
    sniffer_id: u32,
    ack_role: u8,
    now_ms: u64,
) -> PendingRoleReconcile {
    let Ok(mut map) = PENDING_ROLE_CHANGES.lock() else {
        return PendingRoleReconcile::None;
    };
    let Some(entry) = map.get(&sniffer_id) else {
        return PendingRoleReconcile::None;
    };
    if ack_role == entry.desired_role {
        map.remove(&sniffer_id);
        return PendingRoleReconcile::Clear;
    }
    if entry.last_send_ms != 0
        && now_ms.saturating_sub(entry.last_send_ms) >= ROLE_SETROLE_MIN_INTERVAL_MS
    {
        PendingRoleReconcile::Resend {
            desired_role: entry.desired_role,
        }
    } else {
        PendingRoleReconcile::None
    }
}

fn stats_gps_inputs(now_ms: u64) -> StatsGpsInputs {
    StatsGpsInputs {
        uart_fresh: gps_uart_fresh(now_ms) as u8,
        rmc_fresh: gps_rmc_fresh(now_ms) as u8,
        gga_fresh: gps_gga_fresh(now_ms) as u8,
        nav_fresh: gps_nav_fresh(now_ms) as u8,
        clock_stale: gps_clock_stale(now_ms) as u8,
        data_fresh: gps_data_fresh(now_ms) as u8,
        rmc_age_ms: gps_rmc_last_parse_age_ms(now_ms).min(u32::MAX as u64),
        gga_age_ms: gps_gga_last_parse_age_ms(now_ms).min(u32::MAX as u64),
        satellites_in_view: gps_satellites_in_view(),
    }
}

pub fn build_gps_snapshot(now_ms: u64, fix: Option<GpsFixInput>) -> GpsSnapshot {
    crate::tier_a_build::build_gps_snapshot(fix, stats_gps_inputs(now_ms))
}

pub fn build_runtime_snapshot_tier_a(
    now_ms: u64,
    boot_ms: u64,
    gps_fix: Option<GpsFixInput>,
) -> RuntimeSnapshotTierA {
    let gps_stats = stats_gps_inputs(now_ms);
    let sniffers: Vec<SnifferSlotInput> = snapshot_sniffers()
        .iter()
        .filter(|s| s.sniffer_id != 0)
        .map(|s| {
            let ch_count = s
                .assigned_chan_count
                .min(wardriver_proto::CHANNELS_MAX as u8) as usize;
            SnifferSlotInput {
                sniffer_id: s.sniffer_id,
                mac: s.mac,
                last_seen_ms: s.last_seen_ms,
                wifi_frag_batches: s.wifi_frag_batches,
                ble_frag_batches: s.ble_frag_batches,
                ap_entries: s.ap_entries,
                ble_entries: s.ble_entries,
                state: s.state,
                ack_cmd_seq: s.ack_cmd_seq,
                role: s.role,
                ble_capable: s.ble_capable,
                five_ghz_capable: s.five_ghz_capable,
                dfs_capable: s.dfs_capable,
                wifi_reasm_dropped: s.wifi_reasm_dropped,
                ble_reasm_dropped: s.ble_reasm_dropped,
                wifi_rows_enqueued: s.wifi_rows_enqueued,
                ble_rows_enqueued: s.ble_rows_enqueued,
                free_heap_bytes: s.free_heap_bytes,
                total_heap_bytes: s.total_heap_bytes,
                cpu_load_pct: s.cpu_load_pct,
                assigned_band: s.assigned_band,
                pinned_channel: s.pinned_channel,
                assigned_channels: s.assigned_channels[..ch_count].to_vec(),
                wifi_bulk_lost_cycles: s.wifi_bulk_lost_cycles,
                ble_bulk_lost_cycles: s.ble_bulk_lost_cycles,
                sniffer_boot_id: s.sniffer_boot_id,
                reboot_count: s.reboot_count,
                fleet_slot: s.fleet_slot,
                adoption_state: s.adoption_state,
                last_dump_visit_ms: s.last_dump_visit_ms,
                last_dump_bulk_ms: s.last_dump_bulk_ms,
                last_dump_post_ms: s.last_dump_post_ms,
                last_dump_done_ms: s.last_dump_done_ms,
                last_dump_queue_ms: s.last_dump_queue_ms,
                last_dump_bulk_xfer_ms: s.last_dump_bulk_xfer_ms,
                time_sync_rtt_ms: s.time_sync_rtt_ms,
                time_sync_one_way_ms: s.time_sync_one_way_ms,
                last_rtt_time_set_ms: s.last_rtt_time_set_ms,
            }
        })
        .collect();
    let detection = snapshot_detection_last();
    let detection_last = DetectionLastInput {
        last_seen_ms: detection.last_seen_ms,
        sniffer_id: detection.sniffer_id,
        src: detection.src,
        wildcard_count: detection.wildcard_count,
        channel: detection.channel,
        rssi_min: detection.rssi_min,
        rssi_max: detection.rssi_max,
        rssi_avg_q8: detection.rssi_avg_q8,
        oui_matched: detection.oui_matched,
        signal_kind: detection.signal_kind,
        detection_method: detection.detection_method,
        ble_company_id: detection.ble_company_id,
    };
    let mobile_surveillance = snapshot_mobile_surveillance_last();
    let mobile_surveillance_last = MobileSurveillanceLastInput {
        last_seen_ms: mobile_surveillance.last_seen_ms,
        sniffer_id: mobile_surveillance.sniffer_id,
        src: mobile_surveillance.src,
        signal_kind: mobile_surveillance.signal_kind,
        detection_method: mobile_surveillance.detection_method,
    };
    let ssid_watch = snapshot_ssid_watch_last();
    let ssid_watch_last = SsidWatchLastInput {
        last_seen_ms: ssid_watch.last_seen_ms,
        sniffer_id: ssid_watch.sniffer_id,
        bssid: ssid_watch.bssid,
        channel: ssid_watch.channel,
        rssi: ssid_watch.rssi,
        ssid: ssid_watch.ssid,
    };
    crate::tier_a_build::build_runtime_snapshot_tier_a(
        now_ms,
        boot_ms,
        scan_allowed() as u8,
        csv_enqueue_allowed() as u8,
        &sniffers,
        detection_last,
        mobile_surveillance_last,
        ssid_watch_last,
        gps_fix,
        gps_stats,
    )
}
