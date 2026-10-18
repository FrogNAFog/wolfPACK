//! WiFi scan batch → WiGLE CSV row encoding.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex, RwLock};

use log::warn;

use crate::config;
use crate::csv;
use crate::gps;
use crate::storage;
use crate::time_sync;
use crate::wifi_scan;
use wardriver_proto::{self as proto, AuthMini, ScanFragV1Header};
use wardriver_stats as stats;

use super::csv_ingest::{csv_batch_gate_allowed, fix_to_geo, CsvArm};
use super::csv_ingest_gps::match_gps_fix_for_obs;
use wardriver_core::gps_anchor::{observed_unix_ms_from_anchor, MAX_FIX_TIMELINE_DELTA_MS};

static EMPTY_BSSID_SET: OnceLock<HashSet<[u8; 6]>> = OnceLock::new();
static EMPTY_SSID_SET: OnceLock<HashSet<String>> = OnceLock::new();

pub(crate) fn empty_bssid_set() -> &'static HashSet<[u8; 6]> {
    EMPTY_BSSID_SET.get_or_init(HashSet::new)
}

pub(crate) fn empty_ssid_set() -> &'static HashSet<String> {
    EMPTY_SSID_SET.get_or_init(HashSet::new)
}

/// WiFi batch → WiGLE CSV (ESP-NOW reassembly complete or HTTP bulk dump).
/// Returns rows accepted into the SD writer queue.
pub(crate) fn wifi_enqueue_csv_for_batch(
    hdr: ScanFragV1Header,
    aps: Vec<proto::ApEntryV1>,
    extras: Vec<proto::ScanEntryExtras>,
    own_management_ap_bssid: &Option<[u8; 6]>,
    wigle_home_for_loop: &Arc<Mutex<Option<[u8; 6]>>>,
    wigle_manual_for_loop: &Arc<RwLock<HashSet<[u8; 6]>>>,
    wigle_ssids_manual_for_loop: &Arc<RwLock<HashSet<String>>>,
    gps_hist: &VecDeque<(i64, gps::Fix)>,
    sd_io: &storage::SdIoHandle,
    mac_table: &mut csv::MacTable,
) -> Option<u32> {
    stats::inc_scan_cycle();
    stats::add_aps_seen(aps.len());
    if csv_batch_gate_allowed(CsvArm::Wifi, hdr.sniffer_scan_end_unix_ms).is_err() {
        return None;
    }
    let mut batch_lines: Vec<Vec<u8>> = Vec::new();
    let home_now_batch = wigle_home_for_loop.lock().ok().and_then(|g| *g);
    let lock_started_ms = stats::now_ms();
    let manual_guard = wigle_manual_for_loop.read().ok();
    let ssid_guard = wigle_ssids_manual_for_loop.read().ok();
    if stats::now_ms().saturating_sub(lock_started_ms) > 2 {
        stats::CONTROL_DENYLIST_LOCK_SLOW.fetch_add(1, Ordering::Relaxed);
    }
    let manual_denylist: &HashSet<[u8; 6]> = match manual_guard.as_deref() {
        Some(s) => s,
        None => empty_bssid_set(),
    };
    let ssid_denylist: &HashSet<String> = match ssid_guard.as_deref() {
        Some(s) => s,
        None => empty_ssid_set(),
    };
    let mut utc_by_second: HashMap<i64, Arc<str>> = HashMap::new();
    for (ap, ex) in aps.into_iter().zip(extras) {
        let skip = config::bssid_in_denylist(
            &ap.bssid,
            own_management_ap_bssid,
            &home_now_batch,
            manual_denylist,
        );
        if skip {
            stats::add_rows_dropped_denylist(1);
            continue;
        }
        let ssid = wifi_scan::ssid_bytes_to_wigle_string(&ap.ssid, (ap.ssid_len as usize).min(32));
        if config::ssid_in_denylist(&ssid, ssid_denylist) {
            stats::add_rows_dropped_denylist(1);
            continue;
        }
        if ap.channel == 0 || wardriver_core::wigle::channel_to_frequency_mhz(ap.channel) == 0 {
            stats::add_rows_dropped_encode(1);
            continue;
        }
        let Some(obs_unix) = observed_unix_ms_from_anchor(
            hdr.sniffer_scan_end_ms,
            hdr.sniffer_scan_end_unix_ms,
            ap.obs_mono_ms,
        ) else {
            continue;
        };
        let Some(fix) = match_gps_fix_for_obs(gps_hist, obs_unix, MAX_FIX_TIMELINE_DELTA_MS) else {
            continue;
        };
        let sec_key = obs_unix / 1000;
        let ts: Arc<str> = match utc_by_second.get(&sec_key) {
            Some(s) => Arc::clone(s),
            None => {
                let Some(s) = time_sync::utc_string_from_unix_ms(obs_unix) else {
                    stats::add_rows_dropped_encode(1);
                    continue;
                };
                let arc = Arc::from(s);
                utc_by_second.insert(sec_key, Arc::clone(&arc));
                arc
            }
        };
        let row = ap_to_wigle_row(hdr, &ap, &ts, Some(fix), &ex);
        let (Some(lat), Some(lon)) = (row.lat, row.lon) else {
            stats::add_rows_dropped_no_hdop(1);
            continue;
        };
        let d = mac_table.consider_wigle_wifi(&row.bssid, lat, lon, ap.rssi);
        if d.new_unique {
            stats::add_unique_wifi_aps(1);
        }
        if !d.allow_row {
            stats::add_rows_dropped_dedup(1);
            continue;
        }
        let mut line = Vec::<u8>::with_capacity(256);
        if let Err(e) = csv::write_wifi_row(&mut line, &row) {
            warn!("control: CSV row encode: {e:#}");
            stats::add_rows_dropped_encode(1);
            continue;
        }
        stats::CONTROL_STAGE_ROW_ENCODE.fetch_add(1, Ordering::Relaxed);
        batch_lines.push(line);
    }
    let n = batch_lines.len() as u32;
    if n == 0 {
        return Some(0);
    }
    if sd_io.try_enqueue_wigle_rows(batch_lines) {
        stats::sniffer_wifi_rows_enqueued(hdr.sniffer_id, n as usize);
        Some(n)
    } else {
        Some(0)
    }
}

fn ap_to_wigle_row(
    _hdr: ScanFragV1Header,
    ap: &proto::ApEntryV1,
    ts: &Arc<str>,
    fix: Option<&gps::Fix>,
    extras: &proto::ScanEntryExtras,
) -> wifi_scan::WigleWifiRow {
    let ssid = wifi_scan::ssid_bytes_to_wigle_string(&ap.ssid, (ap.ssid_len as usize).min(32));
    let auth_mode = wardriver_core::wigle::authmini_to_caps(ap.auth);
    let frequency_mhz = csv::channel_to_frequency_mhz(ap.channel);
    let rcois = if extras.rcoi_ie_payload.is_empty() {
        String::new()
    } else {
        wardriver_core::wigle_rcoi::rcois_from_ie_payload(&extras.rcoi_ie_payload)
    };

    let (lat, lon, altitude_m, accuracy_m) =
        match fix.filter(|f| wifi_scan::is_fix_usable_for_location(f)) {
            Some(f) => fix_to_geo(f),
            None => (None, None, None, None),
        };

    wifi_scan::WigleWifiRow {
        bssid: ap.bssid,
        ssid,
        auth_mode,
        first_seen_utc: Arc::clone(ts),
        channel: ap.channel,
        frequency_mhz,
        rssi: ap.rssi,
        lat,
        lon,
        altitude_m,
        accuracy_m,
        rcois,
        // WiGLE `MfgrId` is for beacon/vendor manufacturer data, not fleet provenance.
        // Sniffer id is visible on the control dashboard (`/api/stats`); keep CSV WiGLE-clean.
        mfgr_id: String::new(),
        row_type: "WIFI",
    }
}
