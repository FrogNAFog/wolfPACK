//! BLE scan batch → WiGLE CSV and LPR-camera / mobile-surveillance alert rows.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use log::warn;

use super::csv_ingest::{csv_batch_gate_allowed, fix_to_geo, CsvArm};
use super::csv_ingest_detection_csv::detection_append_ble_followup_detection;
use super::csv_ingest_gps::{
    choose_fix_from_history, detection_fix_for_observation, match_gps_fix_for_obs,
};
use super::espnow_rx::ReasmBle;
use crate::csv;
use crate::detection_alert_log::DetectionAlertLog;
use crate::detection_ble_detect::LprCameraBleDetector;
use crate::gps;
use crate::storage;
use crate::time_sync;
use crate::wifi_scan;
use wardriver_core::detection::{
    is_mobile_surveillance_ble_method, DetectionBleMethod, DetectionSignalKind,
};
use wardriver_core::gps_anchor::{observed_unix_ms_from_anchor, MAX_FIX_TIMELINE_DELTA_MS};
use wardriver_proto::{self as proto, BleEntryV1, BleScanFragV1Header};
use wardriver_stats as stats;

fn ble_entry_to_row(
    _hdr: proto::BleScanFragV1Header,
    e: &BleEntryV1,
    ts: &Arc<str>,
    fix: Option<&gps::Fix>,
    extras: &proto::ScanEntryExtras,
) -> wifi_scan::WigleBleRow {
    let end = e.name.iter().position(|&x| x == 0).unwrap_or(e.name.len());
    let name = String::from_utf8_lossy(&e.name[..end]).trim().to_string();

    let (lat, lon, altitude_m, accuracy_m) =
        match fix.filter(|f| wifi_scan::is_fix_usable_for_location(f)) {
            Some(f) => fix_to_geo(f),
            None => (None, None, None, None),
        };

    let mfgr_id = if e.company_id != 0 {
        // WiGLE v1.6 Bluetooth rows: MfgrId is the Bluetooth manufacturer ID.
        // Use decimal to match WiGLE examples.
        e.company_id.to_string()
    } else {
        String::new()
    };

    let (capabilities, frequency_code) = match extras.ble_cod {
        Some(cod) => wardriver_core::wigle_ble::ble_csv_fields_from_cod(cod[0], cod[1], cod[2]),
        None => wardriver_core::wigle_ble::ble_csv_fields_no_cod(),
    };

    wifi_scan::WigleBleRow {
        addr: e.addr,
        name,
        capabilities,
        first_seen_utc: Arc::clone(ts),
        channel: None,
        frequency_code: Some(frequency_code),
        rssi: e.rssi,
        lat,
        lon,
        altitude_m,
        accuracy_m,
        rcois: String::new(),
        mfgr_id,
        row_type: "BLE",
    }
}

/// Per-reassembly context shared by the detection-entry and WiGLE-row helpers.
struct BleReassemblyCtx<'a> {
    hdr: &'a BleScanFragV1Header,
    detection_ignore_ref: &'a HashSet<[u8; 6]>,
    now_ms_det: u64,
    lpr_camera_ble: &'a mut LprCameraBleDetector,
    sd_io: &'a storage::SdIoHandle,
    gps_hist: &'a VecDeque<(i64, gps::Fix)>,
    gps_fix: &'a Arc<Mutex<Option<gps::Fix>>>,
    detection_dis_b: u32,
    mac_table: &'a mut csv::MacTable,
    detection_alert_log_ble: &'a mut DetectionAlertLog,
    alert_hub: &'a Arc<crate::alert_feedback::AlertHub>,
}

/// One detection entry in the reassembly: compute the alert, fire the surveillance /
/// camera trigger, record the detection stats, append the detection alert row, remember
/// it in the alert log, and (when no alert) attempt a detection follow-up.
fn process_ble_detection_entry(ctx: &mut BleReassemblyCtx<'_>, e: &BleEntryV1) {
    let alert = if ctx.detection_alert_log_ble.contains_mac(&e.addr) {
        None
    } else {
        ctx.lpr_camera_ble.ingest(
            ctx.hdr.sniffer_id,
            ctx.now_ms_det,
            e,
            ctx.detection_ignore_ref,
            ctx.detection_dis_b,
        )
    };
    if let Some(a) = alert {
        if is_mobile_surveillance_ble_method(a.method) {
            ctx.alert_hub.trigger_with_title(
                crate::alert_feedback::AlertKind::Surveillance,
                csv::mobile_surveillance_oled_title(a.method),
                None,
            );
        } else {
            ctx.alert_hub
                .trigger(crate::alert_feedback::AlertKind::Camera, None);
        }
        let rssi_q8 = (a.rssi as i16) << 8;
        if is_mobile_surveillance_ble_method(a.method) {
            stats::record_mobile_surveillance_alert(stats::MobileSurveillanceLast {
                last_seen_ms: ctx.now_ms_det,
                sniffer_id: a.sniffer_id,
                src: a.src,
                signal_kind: DetectionSignalKind::Ble.as_u8(),
                detection_method: a.method,
            });
        } else {
            stats::record_detection_alert(stats::DetectionLast {
                last_seen_ms: ctx.now_ms_det,
                sniffer_id: a.sniffer_id,
                src: a.src,
                wildcard_count: 0,
                channel: 0,
                rssi_min: a.rssi,
                rssi_max: a.rssi,
                rssi_avg_q8: rssi_q8,
                oui_matched: u8::from(a.method == DetectionBleMethod::MacOuiInfrastructure.as_u8()),
                signal_kind: DetectionSignalKind::Ble.as_u8(),
                detection_method: a.method,
                ble_company_id: a.company_id,
            });
        }
        let append_ble_alert_row = || {
            if !stats::csv_enqueue_allowed() {
                return;
            }
            let obs_unix = observed_unix_ms_from_anchor(
                ctx.hdr.sniffer_scan_end_ms,
                ctx.hdr.sniffer_scan_end_unix_ms,
                e.obs_mono_ms,
            );
            let fix_hist = obs_unix.and_then(|t| {
                choose_fix_from_history(ctx.gps_hist, t, MAX_FIX_TIMELINE_DELTA_MS)
                    .filter(|f| wifi_scan::is_fix_usable_for_location(f))
            });
            let fix_snapshot = ctx.gps_fix.lock().ok().and_then(|g| g.clone());
            let fix_use = fix_hist.or(fix_snapshot);
            let (lat, lon, altitude_m, accuracy_m) = match fix_use
                .as_ref()
                .filter(|f| wifi_scan::is_fix_usable_for_location(f))
            {
                Some(f) => fix_to_geo(f),
                None => (None, None, None, None),
            };
            let ts_s = obs_unix
                .and_then(|t| time_sync::utc_string_from_unix_ms(t))
                .or_else(|| time_sync::now_utc_string());
            let Some(ts) = ts_s else {
                return;
            };
            let ts: Arc<str> = Arc::from(ts);
            let rssi_q8 = (a.rssi as i16) << 8;
            let (row_type, session) =
                csv::detection_session_for_method(a.method, DetectionSignalKind::Ble.as_u8());
            let drow = csv::DetectionAlertCsvRow {
                row_type,
                detection_method_id: a.method,
                signal_kind: DetectionSignalKind::Ble.as_u8(),
                first_seen_utc: ts.as_ref(),
                lat,
                lon,
                altitude_m,
                accuracy_m,
                mac: a.src,
                ssid: "",
                auth_mode: "",
                channel: 0,
                frequency_mhz: 0,
                rssi: a.rssi,
                rssi_min: a.rssi,
                rssi_max: a.rssi,
                rssi_avg_q8: rssi_q8,
                wildcard_count: 0,
                distinct_ch: 0,
                rcois: "",
                mfgr_id: "",
            };
            let _ = csv::enqueue_detection_session_row(ctx.sd_io, &drow, session);
        };
        append_ble_alert_row();
        ctx.detection_alert_log_ble.remember(
            a.src,
            a.method,
            DetectionSignalKind::Ble.as_u8(),
            is_mobile_surveillance_ble_method(a.method),
        );
    } else if let Some(obs_unix) = observed_unix_ms_from_anchor(
        ctx.hdr.sniffer_scan_end_ms,
        ctx.hdr.sniffer_scan_end_unix_ms,
        e.obs_mono_ms,
    ) {
        if let Some(fix) = detection_fix_for_observation(ctx.gps_hist, ctx.gps_fix, Some(obs_unix))
        {
            if let Some(meta) =
                ctx.detection_alert_log_ble
                    .try_followup(&e.addr, fix.lat, fix.lon, ctx.mac_table)
            {
                detection_append_ble_followup_detection(
                    ctx.hdr,
                    e,
                    &meta,
                    &fix,
                    Some(obs_unix),
                    ctx.sd_io,
                );
            }
        }
    }
}

/// Build the WiGLE rows for one reassembly (batch), counting the drop counters and
/// enqueueing the batch into the SD writer. Returns the number of rows enqueued.
fn build_ble_wigle_rows(ctx: &mut BleReassemblyCtx<'_>, entry: &mut ReasmBle) -> u32 {
    if csv_batch_gate_allowed(CsvArm::Ble, ctx.hdr.sniffer_scan_end_unix_ms).is_err() {
        return 0;
    }
    let mut batch_lines: Vec<Vec<u8>> = Vec::new();
    let mut extras = std::mem::take(&mut entry.extras);
    for (e, ex) in entry.entries.drain(..).zip(extras.drain(..)) {
        let Some(obs_unix) = observed_unix_ms_from_anchor(
            ctx.hdr.sniffer_scan_end_ms,
            ctx.hdr.sniffer_scan_end_unix_ms,
            e.obs_mono_ms,
        ) else {
            continue;
        };
        let Some(fix) = match_gps_fix_for_obs(ctx.gps_hist, obs_unix, MAX_FIX_TIMELINE_DELTA_MS)
        else {
            continue;
        };
        let Some(ts) = time_sync::utc_string_from_unix_ms(obs_unix) else {
            stats::add_rows_dropped_encode(1);
            continue;
        };
        let ts: Arc<str> = Arc::from(ts);
        let row = ble_entry_to_row(*ctx.hdr, &e, &ts, Some(&fix), &ex);
        let (Some(lat), Some(lon)) = (row.lat, row.lon) else {
            stats::add_rows_dropped_no_hdop(1);
            continue;
        };
        let d = ctx
            .mac_table
            .consider_wigle_ble(&row.addr, lat, lon, e.rssi);
        if d.new_unique {
            stats::add_unique_ble_devices(1);
        }
        if !d.allow_row {
            stats::add_rows_dropped_dedup(1);
            continue;
        }
        let mut line = Vec::<u8>::with_capacity(256);
        if let Err(err) = csv::write_ble_row(&mut line, &row) {
            warn!("control: BLE CSV encode: {err:#}");
            stats::add_rows_dropped_encode(1);
            continue;
        }
        stats::CONTROL_STAGE_ROW_ENCODE.fetch_add(1, Ordering::Relaxed);
        batch_lines.push(line);
    }
    if batch_lines.is_empty() {
        return 0;
    }
    let n = batch_lines.len() as u32;
    if ctx.sd_io.try_enqueue_wigle_rows(batch_lines) {
        stats::sniffer_ble_rows_enqueued(ctx.hdr.sniffer_id, n as usize);
        n
    } else {
        0
    }
}

/// BLE scan batch reassembled: Flock BLE → `det-*.detectioncsv`; mobile surveillance (Axon **35**) → `Axon-*.mdetectioncsv`.
/// Returns WiGLE BLE rows accepted into the SD writer queue.
pub fn process_completed_ble_reassembly(
    hdr: BleScanFragV1Header,
    mut entry: ReasmBle,
    detection_ignore_ref: &HashSet<[u8; 6]>,
    now_ms_det: u64,
    lpr_camera_ble: &mut LprCameraBleDetector,
    sd_io: &storage::SdIoHandle,
    gps_hist: &VecDeque<(i64, gps::Fix)>,
    gps_fix: &Arc<Mutex<Option<gps::Fix>>>,
    detection_dis_b: u32,
    mac_table: &mut csv::MacTable,
    detection_alert_log_ble: &mut DetectionAlertLog,
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
) -> u32 {
    let mut ctx = BleReassemblyCtx {
        hdr: &hdr,
        detection_ignore_ref,
        now_ms_det,
        lpr_camera_ble,
        sd_io,
        gps_hist,
        gps_fix,
        detection_dis_b,
        mac_table,
        detection_alert_log_ble,
        alert_hub,
    };
    for e in &entry.entries {
        if ctx.detection_ignore_ref.contains(&e.addr) {
            continue;
        }
        process_ble_detection_entry(&mut ctx, e);
    }
    build_ble_wigle_rows(&mut ctx, &mut entry)
}
