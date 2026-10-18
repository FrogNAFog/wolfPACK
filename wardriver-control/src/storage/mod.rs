//! SD card storage layout and session file handling.
//!
//! This module assumes the SD card is mounted as a FAT filesystem at
//! [`MOUNT_POINT`]. Mounting is done in [`crate::control`] using `esp-idf-svc`
//! `Fatfs` + `MountedFatfs` (see `esp-idf-svc` example `sd_spi.rs`).
//!
//! **Tier 3 SDIO:** all `std::fs` on [`MOUNT_POINT`] lives in [`sd_vfs`] and runs
//! only on the `sd-io` worker thread. Other code uses [`SdIoHandle`] RPC or async
//! enqueue. See `scripts/check-no-sd-fs.sh`.
//!
//! Responsibilities here:
//! - Directory layout (`pending/`, `uploaded/`).
//! - Session filename generation (worker thread via [`sd_sessions`]).
//! - Rows are written only after the worker opens a WiGLE session;
//!   [`append_diag_log_line`] and CSV row enqueue go through [`SdIoHandle`].
//! - **Durability:** worker `flush()` + periodic `sync_data()` on session files.
//!
//! It does not upload; uploader reads WiGLE-format `*.csv` from
//! `{MOUNT_POINT}/wolfPACK/wigle/pending/` via [`SdIoHandle`] RPC.
//! Flock detection files go to `wolfPACK/flock/detections/` (`det-*.detectioncsv`).
//! Mobile surveillance (PACK Axon OUI) uses `wolfPACK/mobile_surveillance/detections/`
//! (`Axon-*.mdetectioncsv`).
//! Optional `wardriver-diag.log` in the `wolfPACK/wigle/` root is written by the
//! control node for heap, deduper sizes, reset reason, and upload failures
//! (see [`append_diag_log_line`]).
//!
//! WiGLE-format session files in `wolfPACK/wigle/pending/` are rotated when the next
//! row would exceed [`WIGLE_CAPTURE_MAX_FILE_BYTES`].

pub(crate) mod sd_boot_snapshot;
mod sd_io;
mod sd_sessions;
mod sd_vfs;

use std::collections::HashSet;
use std::io::Result as IoResult;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) use sd_boot_snapshot::{prepare_sd_boot_before_worker_spawn, take_boot_sd_snapshot};
pub use sd_io::{
    configure_sd_writer_queue_capacity, register_sd_io_handle, sd_io_handle,
    sd_writer_queue_capacity, spawn_sd_io_worker, spawn_sd_writer, SdIoHandle, SdIoRpcOp,
    SdIoRpcResult, SdWriterHandle, SD_IO_RPC_CAPACITY, SD_IO_RPC_WAIT_MS, SD_WRITER_QUEUE_CAPACITY,
};

pub const MOUNT_POINT: &str = "/sdcard";

/// All control-node SD data (WiGLE, LPR-camera, exclusions, preferences) lives under this directory.
pub const DIR_SD_DATA_ROOT: &str = "wolfPACK";

#[inline]
#[must_use]
pub fn sd_data_root() -> PathBuf {
    Path::new(MOUNT_POINT).join(DIR_SD_DATA_ROOT)
}

/// WiFi probe recon pcapng output directory (`wolfPACK/recon/wifiprobes/`).
#[must_use]
pub fn recon_wifiprobes_dir() -> PathBuf {
    sd_data_root().join("recon").join("wifiprobes")
}

/// Preferences JSON (Flock masks, future keys): `wolfPACK/config/preferences.json`.
#[must_use]
pub fn control_preferences_path() -> PathBuf {
    sd_data_root().join("config").join("preferences.json")
}

/// Operator-edited channel plan: `wolfPACK/config/channel_plan.txt`.
#[must_use]
pub fn channel_plan_path() -> PathBuf {
    sd_data_root().join("config").join("channel_plan.txt")
}

/// Atomic write of the channel plan file. Creates `config/` if missing.
pub fn write_channel_plan_text(sd_io: &SdIoHandle, text: &str) -> IoResult<()> {
    if !wardriver_stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire) {
        return Err(sd_not_mounted());
    }
    sd_io.rpc_write_atomic_text(channel_plan_path(), text.as_bytes().to_vec())
}

pub const DIR_WIGLE: &str = "wigle";
pub const DIR_PENDING: &str = "pending";
pub const DIR_UPLOADED: &str = "uploaded";

pub const WIGLE_CAPTURE_MAX_FILE_BYTES: u64 = 49 * 1024 * 1024;
pub const WIGLE_CAPTURE_EXT: &str = "csv";
pub const WIGLE_DEFAULT_FILENAME: &str = "wardrive.csv";

#[inline]
pub fn is_wigle_capture_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(e) if e.eq_ignore_ascii_case(WIGLE_CAPTURE_EXT) || e.eq_ignore_ascii_case("wiglecsv")
    )
}

pub const DIR_FLOCK: &str = "flock";
pub const DIR_FLOCK_DETECTIONS: &str = "detections";
pub const DIR_REMOTE_ID: &str = "remote_id";
pub const DIR_MOBILE_SURVEILLANCE: &str = "mobile_surveillance";

pub const RELPATH_WIGLE_MANUAL_BSSID_IGNORE: &str =
    "exclusions/wigle/WiGLE_manual_BSSID_ignore_list.txt";
pub const RELPATH_WIGLE_MANUAL_SSID_IGNORE: &str =
    "exclusions/wigle/WiGLE_manual_SSID_ignore_list.txt";
pub const RELPATH_FLOCK_CAMERA_FALSE_POSITIVE_IGNORE: &str =
    "exclusions/flock/Flock_camera_false_positive_ignore_list.txt";
/// Operator SSID watchlist (beacon match triggers alert; no separate log file).
pub const RELPATH_SSID_WATCHLIST: &str = "config/ssid_watchlist.txt";

pub const WIGLE_MANUAL_IGNORE_FILE_HEADER: &str = concat!(
    "# WiGLE CSV row filter: manual BSSID ignore list.\n",
    "# These BSSIDs are omitted from WiGLE CSV rows; this file is NOT used for LPR-camera detection.\n",
    "# One MAC per line (aa:bb:cc:dd:ee:ff, dashes, or 12 hex digits).\n\n",
);

pub const WIGLE_MANUAL_SSID_IGNORE_FILE_HEADER: &str = concat!(
    "# WiGLE CSV row filter: manual SSID ignore list.\n",
    "# APs with these SSIDs are omitted from WiGLE CSV rows and sniffer scan reports.\n",
    "# One SSID per line (max 32 UTF-8 bytes). Total must fit one ESP-NOW frame.\n\n",
);

pub const FLOCK_IGNORE_FILE_HEADER: &str = concat!(
    "# LPR-camera: STA MACs ignored for probe-burst detection (false positives).\n",
    "# This list does NOT affect WiGLE CSV; see exclusions/wigle/ for WiGLE manual ignores.\n",
    "# One MAC per line.\n\n",
);

pub const SSID_WATCHLIST_FILE_HEADER: &str = concat!(
    "# SSID watchlist: alert when a sniffer hears an AP beacon with a listed SSID.\n",
    "# Triggers buzzer + dashboard banner only (no separate log file).\n",
    "# One SSID per line (max 32 UTF-8 bytes). Total must fit one ESP-NOW frame.\n\n",
);

pub fn wigle_manual_bssid_ignore_path() -> PathBuf {
    sd_data_root().join(RELPATH_WIGLE_MANUAL_BSSID_IGNORE)
}

pub fn wigle_manual_ssid_ignore_path() -> PathBuf {
    sd_data_root().join(RELPATH_WIGLE_MANUAL_SSID_IGNORE)
}

pub fn flock_camera_false_positive_ignore_path() -> PathBuf {
    sd_data_root().join(RELPATH_FLOCK_CAMERA_FALSE_POSITIVE_IGNORE)
}

pub fn ssid_watchlist_path() -> PathBuf {
    sd_data_root().join(RELPATH_SSID_WATCHLIST)
}

pub(crate) fn format_mac_colon(m: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}

pub(crate) fn validate_ssid_denylist_pack(set: &HashSet<String>) -> Result<(), String> {
    let mut ssids: Vec<String> = set.iter().cloned().collect();
    ssids.sort();
    wardriver_proto::pack_ssid_denylist(&ssids)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Same size limits as WiGLE SSID denylist; distinct ESP-NOW frame type on the wire.
pub(crate) fn validate_ssid_watchlist_pack(set: &HashSet<String>) -> Result<(), String> {
    let mut ssids: Vec<String> = set.iter().cloned().collect();
    ssids.sort();
    wardriver_proto::pack_ssid_watchlist(&ssids)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn parse_ssid_ignore_file_text(text: &str) -> Result<HashSet<String>, String> {
    let filtered: String = text
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                None
            } else {
                Some(t)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let set = crate::config::parse_exclude_ssid_lines(&filtered)?;
    validate_ssid_denylist_pack(&set)?;
    Ok(set)
}

/// Parse operator SSID watchlist text (same line rules as WiGLE SSID exclusions).
pub fn parse_ssid_watchlist_file_text(text: &str) -> Result<HashSet<String>, String> {
    let filtered: String = text
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                None
            } else {
                Some(t)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let set = crate::config::parse_exclude_ssid_lines(&filtered)?;
    validate_ssid_watchlist_pack(&set)?;
    Ok(set)
}

pub fn parse_mac_ignore_file_text(text: &str) -> Result<HashSet<[u8; 6]>, String> {
    let filtered: String = text
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                None
            } else {
                Some(t)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    crate::config::parse_exclude_mac_lines(&filtered)
}

/// Write sorted SSID list with header via SD worker RPC.
pub fn save_ssid_ignore_list(
    sd_io: &SdIoHandle,
    path: &Path,
    header: &str,
    set: &HashSet<String>,
) -> IoResult<()> {
    validate_ssid_denylist_pack(set)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut ssids: Vec<String> = set.iter().cloned().collect();
    ssids.sort();
    let mut body = String::new();
    body.push_str(header);
    for s in &ssids {
        body.push_str(s);
        body.push('\n');
    }
    sd_io.rpc_write_atomic_text(path.to_path_buf(), body.into_bytes())
}

/// Persist SSID watchlist (detections page) via SD worker RPC.
pub fn save_ssid_watchlist(sd_io: &SdIoHandle, path: &Path, set: &HashSet<String>) -> IoResult<()> {
    validate_ssid_watchlist_pack(set)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut ssids: Vec<String> = set.iter().cloned().collect();
    ssids.sort();
    let mut body = String::new();
    body.push_str(SSID_WATCHLIST_FILE_HEADER);
    for s in &ssids {
        body.push_str(s);
        body.push('\n');
    }
    sd_io.rpc_write_atomic_text(path.to_path_buf(), body.into_bytes())
}

/// Write sorted MAC list with header via SD worker RPC.
pub fn save_mac_ignore_list(
    sd_io: &SdIoHandle,
    path: &Path,
    header: &str,
    set: &HashSet<[u8; 6]>,
) -> IoResult<()> {
    let body = mac_ignore_list_body(header, set)?;
    sd_io.rpc_write_atomic_text(path.to_path_buf(), body.into_bytes())
}

fn mac_ignore_list_body(header: &str, set: &HashSet<[u8; 6]>) -> IoResult<String> {
    let mut macs: Vec<[u8; 6]> = set.iter().copied().collect();
    macs.sort();
    let mut body = String::new();
    body.push_str(header);
    for m in &macs {
        body.push_str(&format_mac_colon(m));
        body.push('\n');
    }
    Ok(body)
}

pub(crate) const DIAG_LOG_RELPATH: &str = "wardriver-diag.log";
pub(crate) const DIAG_LOG_MAX_BYTES: u64 = 512 * 1024;
pub(crate) const DIAG_LOG_KEEP_FILES: usize = 5;

/// Append one UTF-8 diag line via the SD I/O worker (adds newline on write).
pub fn append_diag_log_line(sd_io: &SdIoHandle, line: &str) -> IoResult<()> {
    if !wardriver_stats::diag_log_enabled() {
        return Ok(());
    }
    if sd_io.try_enqueue_diag_line(line.to_string()) {
        Ok(())
    } else if let Err(e) = sd_io.rpc_append_diag_line(line) {
        wardriver_stats::diag_log_line_err();
        Err(e)
    } else {
        Ok(())
    }
}

/// Append a diag line when the caller has no `&SdIoHandle` (upload failures, GPS recovery).
///
/// Tries async enqueue first, then blocking RPC on the global handle. Never touches VFS directly.
pub fn append_diag_log_line_best_effort(line: &str) -> IoResult<()> {
    if !wardriver_stats::diag_log_enabled() {
        return Ok(());
    }
    if let Some(sd_io) = sd_io_handle() {
        if sd_io.try_enqueue_diag_line(line.to_string()) {
            return Ok(());
        }
        return sd_io.rpc_append_diag_line(line);
    }
    wardriver_stats::diag_log_line_err();
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "sd-io handle not registered",
    ))
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub pending: PathBuf,
    pub uploaded: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let root = sd_data_root().join(DIR_WIGLE);
        let pending = root.join(DIR_PENDING);
        let uploaded = root.join(DIR_UPLOADED);
        Self {
            root,
            pending,
            uploaded,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DetectionPaths {
    pub pending: PathBuf,
}

impl DetectionPaths {
    pub fn new() -> Self {
        let pending = sd_data_root().join(DIR_FLOCK).join(DIR_FLOCK_DETECTIONS);
        Self { pending }
    }
}

#[derive(Clone, Debug)]
pub struct MdetectionPaths {
    pub pending: PathBuf,
}

impl MdetectionPaths {
    pub fn new() -> Self {
        let pending = sd_data_root()
            .join(DIR_MOBILE_SURVEILLANCE)
            .join(DIR_FLOCK_DETECTIONS);
        Self { pending }
    }
}

#[derive(Clone, Debug)]
pub struct RemoteIdPaths {
    pub pending: PathBuf,
}

impl RemoteIdPaths {
    pub fn new() -> Self {
        let pending = sd_data_root()
            .join(DIR_REMOTE_ID)
            .join(DIR_FLOCK_DETECTIONS);
        Self { pending }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CleanupResult {
    pub examined: u32,
    pub deleted: u32,
    pub errors: u32,
}

static BOOT_WIGLE_CLEANUP_PENDING: AtomicBool = AtomicBool::new(false);

/// Defer header-only WiGLE CSV cleanup until after boot finishes (avoids blocking main during mount).
pub fn request_boot_wigle_cleanup() {
    BOOT_WIGLE_CLEANUP_PENDING.store(true, Ordering::Release);
}

/// Run deferred boot cleanup once; called from the control main loop.
pub fn run_deferred_boot_wigle_cleanup(sd_io: &SdIoHandle) {
    if !BOOT_WIGLE_CLEANUP_PENDING.swap(false, Ordering::AcqRel) {
        return;
    }
    let cleanup = cleanup_header_only_wiglecsv_files(sd_io);
    wardriver_stats::record_sd_cleanup(cleanup.examined, cleanup.deleted, cleanup.errors);
    if cleanup.deleted > 0 || cleanup.errors > 0 {
        log::info!(
            "wolfpack/sd: boot cleanup examined={} deleted={} errors={}",
            cleanup.examined,
            cleanup.deleted,
            cleanup.errors
        );
    }
}

/// Best-effort cleanup of header-only WiGLE capture files (worker RPC).
pub fn cleanup_header_only_wiglecsv_files(sd_io: &SdIoHandle) -> CleanupResult {
    match sd_io.submit_and_wait_timeout(SdIoRpcOp::CleanupHeaderOnlyWigle, Duration::from_secs(120))
    {
        SdIoRpcResult::Cleanup(c) => c,
        _ => CleanupResult::default(),
    }
}

fn sd_not_mounted() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, "sd not mounted")
}

/// Read preferences JSON via SD worker RPC.
pub fn read_preferences_text(sd_io: &SdIoHandle) -> IoResult<Option<String>> {
    sd_io.rpc_read_text(control_preferences_path())
}

/// Atomic preferences write via SD worker RPC.
pub fn write_preferences_text(sd_io: &SdIoHandle, body: &str) -> IoResult<()> {
    sd_io.rpc_write_atomic_text(control_preferences_path(), body.as_bytes().to_vec())
}

/// Refresh cached pending CSV count (boot snapshot and after upload runs).
pub fn refresh_upload_pending_csv_count(sd_io: &SdIoHandle) {
    let _ = sd_io.rpc_count_pending_csv();
}
