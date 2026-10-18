//! Boot-time SD reads on the control main task before the sd-io worker pthread starts.
//!
//! FAT/VFS is not safe for concurrent access from multiple pthreads. After mount and
//! before [`super::spawn_sd_io_worker`], only the main boot task touches `/sdcard`; the
//! worker handles runtime I/O afterward.

use std::sync::Mutex;

use crate::time_sync;
use log::warn;

use super::sd_vfs;
use super::{
    channel_plan_path, control_preferences_path, flock_camera_false_positive_ignore_path,
    ssid_watchlist_path, wigle_manual_bssid_ignore_path, wigle_manual_ssid_ignore_path,
};
use wardriver_shared::boot_diag;
use wardriver_stats as stats;

static BOOT_SNAPSHOT: Mutex<Option<BootSdSnapshot>> = Mutex::new(None);

/// Text blobs read from SD during boot (main applies in RAM after worker spawn).
#[derive(Clone, Debug, Default)]
pub(crate) struct BootSdSnapshot {
    pub preferences: Option<String>,
    pub channel_plan: Option<String>,
    pub wigle_bssid_ignore: Option<String>,
    pub wigle_ssid_ignore: Option<String>,
    pub detection_ignore: Option<String>,
    pub ssid_watchlist: Option<String>,
}

pub(crate) fn take_boot_sd_snapshot() -> Option<BootSdSnapshot> {
    BOOT_SNAPSHOT.lock().ok().and_then(|mut g| g.take())
}

pub(crate) fn store_boot_sd_snapshot(snap: BootSdSnapshot) {
    if let Ok(mut g) = BOOT_SNAPSHOT.lock() {
        *g = Some(snap);
    }
}

/// Main boot task only: layout, read config/exclusion files, BOOT diag line (no worker yet).
pub(crate) fn prepare_sd_boot_before_worker_spawn() {
    if let Err(e) = sd_vfs::ensure_standard_layout() {
        warn!("control: SD layout: {e:#}");
    }
    let snap = BootSdSnapshot {
        preferences: sd_vfs::read_text(&control_preferences_path())
            .ok()
            .flatten(),
        channel_plan: sd_vfs::read_text(&channel_plan_path()).ok().flatten(),
        wigle_bssid_ignore: sd_vfs::read_text(&wigle_manual_bssid_ignore_path())
            .ok()
            .flatten(),
        wigle_ssid_ignore: sd_vfs::read_text(&wigle_manual_ssid_ignore_path())
            .ok()
            .flatten(),
        detection_ignore: sd_vfs::read_text(&flock_camera_false_positive_ignore_path())
            .ok()
            .flatten(),
        ssid_watchlist: sd_vfs::read_text(&ssid_watchlist_path()).ok().flatten(),
    };
    store_boot_sd_snapshot(snap);
    stats::set_upload_pending_csv_count(sd_vfs::count_pending_wigle_csvs());
    super::request_boot_wigle_cleanup();
    let boot_line = format!(
        "BOOT utc={} t_ms={} ver={} rst={} rst_n={} free_heap={} min_ever_free_heap={} free_internal_heap={} free_spiram_heap={}",
        time_sync::now_utc_string().unwrap_or_else(|| "-".into()),
        stats::now_ms(),
        env!("CARGO_PKG_VERSION"),
        boot_diag::reset_reason_str(),
        boot_diag::reset_reason_raw(),
        stats::free_heap_bytes(),
        stats::min_ever_free_heap_bytes(),
        stats::free_internal_heap_bytes(),
        stats::free_spiram_heap_bytes(),
    );
    if let Err(e) = sd_vfs::write_diag_log_line(&boot_line) {
        warn!("control: SD boot marker write failed: {e:#}");
    }
}
