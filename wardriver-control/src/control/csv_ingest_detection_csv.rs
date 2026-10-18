//! Flock / mobile-surveillance follow-up rows on Detection and mdetection sessions.

use std::sync::Arc;

use crate::csv;
use crate::gps;
use crate::storage;
use crate::time_sync;
use crate::wifi_scan;
use std::sync::atomic::Ordering;
use wardriver_proto::{
    ApEntryV1, BleEntryV1, BleScanFragV1Header, ProbeReqBurstV1, ProbeReqFragV1Header,
    ScanFragV1Header,
};
use wardriver_stats as stats;

use super::csv_ingest::fix_to_geo;
use super::csv_ingest_gps::detection_fix_for_observation;
use crate::detection_alert_log::DetectionAlertLog;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use wardriver_core::gps_anchor::observed_unix_ms_from_anchor;

pub(crate) fn detection_append_wifi_followup_detection(
    _hdr: &ProbeReqFragV1Header,
    e: &ProbeReqBurstV1,
    meta: &crate::detection_alert_log::DetectionAlertMeta,
    fix: &gps::Fix,
    obs_unix: Option<i64>,
    sd_io: &storage::SdIoHandle,
) -> bool {
    if !stats::csv_enqueue_allowed() {
        return false;
    }
    let (lat, lon, altitude_m, accuracy_m) = fix_to_geo(fix);
    let ts_s = obs_unix
        .and_then(|t| time_sync::utc_string_from_unix_ms(t))
        .or_else(|| time_sync::now_utc_string());
    let Some(ts) = ts_s else {
        return false;
    };
    let ts: Arc<str> = Arc::from(ts);
    let rssi_avg = (e.rssi_avg_q8 as f32) / 256.0;
    let ch = e.channel;
    let (row_type, session) = csv::detection_session_for_method(meta.method_id, meta.signal_kind);
    let drow = csv::DetectionAlertCsvRow {
        row_type,
        detection_method_id: meta.method_id,
        signal_kind: meta.signal_kind,
        first_seen_utc: ts.as_ref(),
        lat,
        lon,
        altitude_m,
        accuracy_m,
        mac: e.src,
        ssid: "",
        auth_mode: "",
        channel: ch,
        frequency_mhz: csv::channel_to_frequency_mhz(ch),
        rssi: rssi_avg.round() as i8,
        rssi_min: e.rssi_min,
        rssi_max: e.rssi_max,
        rssi_avg_q8: e.rssi_avg_q8,
        wildcard_count: e.wildcard_count as u16,
        distinct_ch: u8::from(e.channel > 0),
        rcois: "",
        mfgr_id: "",
    };
    if csv::enqueue_detection_session_row(sd_io, &drow, session) {
        stats::DETECTION_FOLLOWUP_ROWS_WRITTEN.fetch_add(1, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// Wi-Fi AP (beacon) follow-ups for mobile-surveillance OUI alerts already in the registry.
pub(crate) fn apply_wifi_ap_ms_followups(
    hdr: &ScanFragV1Header,
    aps: &[ApEntryV1],
    detection_ignore: &HashSet<[u8; 6]>,
    gps_hist: &VecDeque<(i64, gps::Fix)>,
    gps_fix: &std::sync::Arc<Mutex<Option<gps::Fix>>>,
    detection_alert_log_wifi: &mut DetectionAlertLog,
    mac_table: &mut csv::MacTable,
    sd_io: &storage::SdIoHandle,
) -> u32 {
    let mut written = 0u32;
    for ap in aps {
        if detection_ignore.contains(&ap.bssid) {
            continue;
        }
        let Some(peek) = detection_alert_log_wifi.meta(&ap.bssid) else {
            continue;
        };
        if !peek.is_mobile_surveillance {
            continue;
        }
        let Some(obs_unix) = observed_unix_ms_from_anchor(
            hdr.sniffer_scan_end_ms,
            hdr.sniffer_scan_end_unix_ms,
            ap.obs_mono_ms,
        ) else {
            continue;
        };
        let Some(fix) = detection_fix_for_observation(gps_hist, gps_fix, Some(obs_unix)) else {
            continue;
        };
        let Some(meta) =
            detection_alert_log_wifi.try_followup(&ap.bssid, fix.lat, fix.lon, mac_table)
        else {
            continue;
        };
        if !stats::csv_enqueue_allowed() {
            continue;
        }
        let (lat, lon, altitude_m, accuracy_m) = fix_to_geo(&fix);
        let Some(ts) =
            time_sync::utc_string_from_unix_ms(obs_unix).or_else(|| time_sync::now_utc_string())
        else {
            continue;
        };
        let ts: Arc<str> = Arc::from(ts);
        let ch = ap.channel;
        let (row_type, session) =
            csv::detection_session_for_method(meta.method_id, meta.signal_kind);
        let drow = csv::DetectionAlertCsvRow {
            row_type,
            detection_method_id: meta.method_id,
            signal_kind: meta.signal_kind,
            first_seen_utc: ts.as_ref(),
            lat,
            lon,
            altitude_m,
            accuracy_m,
            mac: ap.bssid,
            ssid: "",
            auth_mode: "",
            channel: ch,
            frequency_mhz: csv::channel_to_frequency_mhz(ch),
            rssi: ap.rssi,
            rssi_min: ap.rssi,
            rssi_max: ap.rssi,
            rssi_avg_q8: (ap.rssi as i16) << 8,
            wildcard_count: 0,
            distinct_ch: u8::from(ch > 0),
            rcois: "",
            mfgr_id: "",
        };
        if csv::enqueue_detection_session_row(sd_io, &drow, session) {
            stats::DETECTION_FOLLOWUP_ROWS_WRITTEN.fetch_add(1, Ordering::Relaxed);
            written = written.saturating_add(1);
        }
    }
    written
}

pub(crate) fn detection_append_ble_followup_detection(
    _hdr: &BleScanFragV1Header,
    e: &BleEntryV1,
    meta: &crate::detection_alert_log::DetectionAlertMeta,
    fix: &gps::Fix,
    obs_unix: Option<i64>,
    sd_io: &storage::SdIoHandle,
) {
    if !stats::csv_enqueue_allowed() {
        return;
    }
    let (row_type, session) = csv::detection_session_for_method(meta.method_id, meta.signal_kind);
    let (lat, lon, altitude_m, accuracy_m) = fix_to_geo(fix);
    let ts_s = obs_unix
        .and_then(|t| time_sync::utc_string_from_unix_ms(t))
        .or_else(|| time_sync::now_utc_string());
    let Some(ts) = ts_s else {
        return;
    };
    let ts: Arc<str> = Arc::from(ts);
    let rssi_q8 = (e.rssi as i16) << 8;
    let drow = csv::DetectionAlertCsvRow {
        row_type,
        detection_method_id: meta.method_id,
        signal_kind: meta.signal_kind,
        first_seen_utc: ts.as_ref(),
        lat,
        lon,
        altitude_m,
        accuracy_m,
        mac: e.addr,
        ssid: "",
        auth_mode: "",
        channel: 0,
        frequency_mhz: 0,
        rssi: e.rssi,
        rssi_min: e.rssi,
        rssi_max: e.rssi,
        rssi_avg_q8: rssi_q8,
        wildcard_count: 0,
        distinct_ch: 0,
        rcois: "",
        mfgr_id: "",
    };
    if csv::enqueue_detection_session_row(sd_io, &drow, session) {
        stats::DETECTION_FOLLOWUP_ROWS_WRITTEN.fetch_add(1, Ordering::Relaxed);
    }
}
