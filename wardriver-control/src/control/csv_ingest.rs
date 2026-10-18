//! WiFi/BLE WiGLE CSV row encoding and GPS timeline matching for scan batches.
//!
//! Implementation is split across `csv_ingest_gps`, `csv_ingest_wifi`, `csv_ingest_ble`, and
//! `csv_ingest_detection_csv`; this module re-exports the control-facing API.

use std::sync::atomic::Ordering;

use crate::gps;
use crate::time_sync;
use crate::wifi_scan;
use wardriver_stats as stats;

pub(crate) use super::csv_ingest_ble::process_completed_ble_reassembly;
pub(crate) use super::csv_ingest_detection_csv::{
    apply_wifi_ap_ms_followups, detection_append_wifi_followup_detection,
};
pub(crate) use super::csv_ingest_gps::{
    choose_fix_from_history, detection_fix_for_observation, gps_hist_can_drain, unix_now_ms,
};
pub(crate) use super::csv_ingest_probe::probe_enqueue_csv_for_batch;
pub(crate) use super::csv_ingest_wifi::{empty_bssid_set, wifi_enqueue_csv_for_batch};

/// Capacity for per-MAC first-alert registry (WiFi and BLE each).
pub(crate) const DETECTION_ALERT_LOG_CAP: usize = 256;

/// CSV batch gate arm: selects which per-arm drop-counter fns to invoke.
pub(crate) enum CsvArm {
    Wifi,
    Ble,
}

/// Reason a CSV batch gate check dropped the batch (one variant per check, in check order).
pub(crate) enum DropReason {
    FleetHold,
    HomeGeofence,
    CsvGate,
    WallClock,
    NoFixTimeline,
}

/// The 5-check CSV batch gate prologue, run in a fixed order.
///
/// Returns `Ok(())` when the batch may proceed, or `Err(DropReason)` naming the check that
/// dropped it. The `arm` selects which per-arm drop-counter fn is invoked for the failing
/// check, so the emitted drop-counter names stay byte-identical per arm.
pub(crate) fn csv_batch_gate_allowed(
    arm: CsvArm,
    sniffer_scan_end_unix_ms: u64,
) -> Result<(), DropReason> {
    if stats::fleet_management_hold() {
        match arm {
            CsvArm::Wifi => stats::add_wifi_batches_dropped_fleet_hold(),
            CsvArm::Ble => stats::add_ble_batches_dropped_fleet_hold(),
        }
        return Err(DropReason::FleetHold);
    }
    if stats::HOME_GEOFENCE_SUPPRESSING.load(Ordering::Acquire) {
        match arm {
            CsvArm::Wifi => stats::add_wifi_batches_dropped_home_geofence(),
            CsvArm::Ble => stats::add_ble_batches_dropped_home_geofence(),
        }
        return Err(DropReason::HomeGeofence);
    }
    if !stats::csv_enqueue_allowed() {
        match arm {
            CsvArm::Wifi => stats::add_wifi_batches_dropped_csv_gate(),
            CsvArm::Ble => stats::add_ble_batches_dropped_csv_gate(),
        }
        return Err(DropReason::CsvGate);
    }
    if !time_sync::wall_clock_trusted() {
        match arm {
            CsvArm::Wifi => stats::add_wifi_batches_dropped_wall_clock(),
            CsvArm::Ble => stats::add_ble_batches_dropped_wall_clock(),
        }
        return Err(DropReason::WallClock);
    }
    if sniffer_scan_end_unix_ms == 0 {
        match arm {
            CsvArm::Wifi => stats::add_wifi_batches_dropped_no_fix_timeline(),
            CsvArm::Ble => stats::add_ble_batches_dropped_no_fix_timeline(),
        }
        return Err(DropReason::NoFixTimeline);
    }
    Ok(())
}

/// Map a GPS fix to the (lat, lon, altitude_m, accuracy_m) 4-tuple used by CSV rows.
pub(crate) fn fix_to_geo(fix: &gps::Fix) -> (Option<f64>, Option<f64>, Option<i32>, Option<f32>) {
    (
        Some(fix.lat),
        Some(fix.lon),
        fix.altitude_m.map(|a| a.round() as i32),
        fix.hdop.map(wifi_scan::hdop_to_accuracy_m),
    )
}
