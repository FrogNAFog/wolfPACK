//! WiGLE/LPR-camera exclusion list reload from SD and sniffer-session state cleanup.

use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use anyhow::{anyhow, Result};
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use log::{info, warn};

use super::espnow_rx::{self, Reasm, ReasmBle, ReasmKey, ReasmProbe};
use crate::config;
use crate::storage::{self, sd_boot_snapshot::BootSdSnapshot, SdIoHandle};

/// Control-side dedup: (sniffer_id, scan_batch_id, seq, frag_idx)
type DedupKey = espnow_rx::DedupKey;

/// Hard reader restart when nav/clock still missing after soft budget or this age.
const GPS_HARD_RECOVERY_MS: u64 = 30_000;
const GPS_SOFT_RECOVERY_MAX_PER_WINDOW: u32 = 3;
const GPS_SOFT_RECOVERY_WINDOW_MS: u64 = 300_000;

/// Apply WiGLE + LPR-camera exclusion lists from a worker boot snapshot (main thread, RAM only).
pub(crate) fn apply_exclusion_lists_from_boot_snapshot(
    nvs: &EspNvsPartition<NvsDefault>,
    snap: &BootSdSnapshot,
    wigle: &Arc<RwLock<HashSet<[u8; 6]>>>,
    wigle_ssids: &Arc<RwLock<HashSet<String>>>,
    detection: &Arc<RwLock<HashSet<[u8; 6]>>>,
    ssid_watch: &Arc<RwLock<HashSet<String>>>,
) -> Result<()> {
    let fp = storage::flock_camera_false_positive_ignore_path();
    let sp = storage::wigle_manual_ssid_ignore_path();

    let detection_set = match snap.detection_ignore.as_deref() {
        Some(txt) => storage::parse_mac_ignore_file_text(txt).map_err(|e| anyhow!(e))?,
        None => HashSet::new(),
    };
    if let Ok(mut w) = detection.write() {
        *w = detection_set;
    }

    let wigle_set = match snap.wigle_bssid_ignore.as_deref() {
        Some(txt) => storage::parse_mac_ignore_file_text(txt).map_err(|e| anyhow!(e))?,
        None => config::wigle_exclude_macs_load(nvs).unwrap_or_default(),
    };
    if let Ok(mut w) = wigle.write() {
        *w = wigle_set;
    }

    let ssid_set = match snap.wigle_ssid_ignore.as_deref() {
        Some(txt) => storage::parse_ssid_ignore_file_text(txt).map_err(|e| anyhow!(e))?,
        None => HashSet::new(),
    };
    if let Ok(mut w) = wigle_ssids.write() {
        *w = ssid_set;
    }

    let watch_set = match snap.ssid_watchlist.as_deref() {
        Some(txt) => storage::parse_ssid_watchlist_file_text(txt).map_err(|e| anyhow!(e))?,
        None => HashSet::new(),
    };
    if let Ok(mut w) = ssid_watch.write() {
        *w = watch_set;
    }
    Ok(())
}

pub(crate) fn try_gps_soft_or_hard_recovery(
    gps_supervisor: &std::sync::Arc<crate::gps::GpsSupervisor>,
    stale_age_ms: u64,
    soft_in_window: &mut u32,
    soft_window_start_ms: &mut u64,
    now_ms: u64,
) {
    if *soft_window_start_ms == 0
        || now_ms.saturating_sub(*soft_window_start_ms) > GPS_SOFT_RECOVERY_WINDOW_MS
    {
        *soft_in_window = 0;
        *soft_window_start_ms = now_ms;
    }
    if *soft_in_window < GPS_SOFT_RECOVERY_MAX_PER_WINDOW {
        gps_supervisor.request_soft_recovery();
        *soft_in_window = soft_in_window.saturating_add(1);
    } else if stale_age_ms > GPS_HARD_RECOVERY_MS
        || *soft_in_window >= GPS_SOFT_RECOVERY_MAX_PER_WINDOW
    {
        if let Err(e) = gps_supervisor.restart_reader() {
            warn!("control: gps hard restart: {e:#}");
        }
        *soft_in_window = 0;
        *soft_window_start_ms = now_ms;
    }
}

/// Clear dedup + reassembly state for one sniffer (e.g. after `sniffer_boot_id` changes).
pub(crate) fn drop_state_for_sniffer(
    sid: u32,
    dedup: &mut IndexMap<DedupKey, u64>,
    dedup_ble: &mut IndexMap<DedupKey, u64>,
    dedup_probe: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, Reasm>,
    reasm_ble: &mut HashMap<ReasmKey, ReasmBle>,
    reasm_probe: &mut HashMap<ReasmKey, ReasmProbe>,
) {
    dedup.retain(|k, _| k.0 != sid);
    dedup_ble.retain(|k, _| k.0 != sid);
    dedup_probe.retain(|k, _| k.0 != sid);
    espnow_rx::drop_reasm_for_sniffer_with_probe(sid, reasm, reasm_ble, reasm_probe);
}
