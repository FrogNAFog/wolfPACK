//! GPS timeline matching for WiGLE/Detection row encoding (shared by WiFi and BLE ingest).
//!
//! Pure cores (`match_gps_fix_hist_index`, `observed_unix_ms_from_anchor`,
//! `MAX_FIX_TIMELINE_DELTA_MS`) live in `wardriver_core::gps_anchor` (host-tested).
//! This module keeps only the `gps::Fix`-typed wrappers (control-only type) and the
//! esp-idf clock read.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::gps;
use crate::wifi_scan;
use esp_idf_sys as sys;
use wardriver_proto as proto;
use wardriver_stats as stats;

use wardriver_core::gps_anchor::{self, GpsFix, MAX_FIX_TIMELINE_DELTA_MS};

pub(crate) fn unix_now_ms() -> Option<u64> {
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, core::ptr::null_mut());
    }
    if tv.tv_sec <= 0 {
        return None;
    }
    let ms = (tv.tv_sec as i64)
        .saturating_mul(1000)
        .saturating_add((tv.tv_usec as i64) / 1000);
    if ms <= 0 {
        return None;
    }
    Some(ms as u64)
}

fn fix_to_gps_fix(f: &gps::Fix) -> GpsFix {
    GpsFix {
        utc_unix_ms: f.utc_unix_ms,
        lat: f.lat,
        lon: f.lon,
        altitude_m: f.altitude_m,
        hdop: f.hdop,
        accuracy_m: None,
        fix_quality: f.fix_quality,
    }
}

fn gps_hist_core(hist: &VecDeque<(i64, gps::Fix)>) -> VecDeque<(i64, GpsFix)> {
    hist.iter().map(|(t, f)| (*t, fix_to_gps_fix(f))).collect()
}

/// `hist` entries are `(sample_unix_ms, Fix)` where `sample_unix_ms` is control wall time at ingest.
pub(crate) fn choose_fix_from_history(
    hist: &VecDeque<(i64, gps::Fix)>,
    observed_unix_ms: i64,
    max_delta_ms: u64,
) -> Option<gps::Fix> {
    let core_hist = gps_hist_core(hist);
    let idx = gps_anchor::match_gps_fix_hist_index(&core_hist, observed_unix_ms, max_delta_ms)?;
    hist.get(idx).map(|(_, f)| f.clone())
}

pub(crate) fn match_gps_fix_for_obs<'a>(
    hist: &'a VecDeque<(i64, gps::Fix)>,
    observed_unix_ms: i64,
    max_delta_ms: u64,
) -> Option<&'a gps::Fix> {
    let core_hist = gps_hist_core(hist);
    let idx = match gps_anchor::match_gps_fix_hist_index(&core_hist, observed_unix_ms, max_delta_ms)
    {
        Some(i) => i,
        None => {
            stats::add_rows_dropped_no_gps_match(1);
            return None;
        }
    };
    hist.get(idx).map(|(_, f)| f)
}

/// True when the newest usable history sample still covers `now_unix_ms` for sniffer off-channel slack.
pub(crate) fn gps_hist_can_drain(hist: &VecDeque<(i64, gps::Fix)>, now_unix_ms: i64) -> bool {
    if now_unix_ms == 0 {
        return false;
    }
    let Some((t, _)) = hist
        .iter()
        .filter(|(_, fix)| wifi_scan::is_fix_usable_for_location(fix))
        .max_by_key(|(t, _)| *t)
    else {
        return false;
    };
    let horizon = t
        .saturating_add(MAX_FIX_TIMELINE_DELTA_MS as i64)
        .saturating_add(proto::SNIFFER_MAX_OFF_CONTROL_MS as i64);
    if horizon >= now_unix_ms {
        return true;
    }
    (now_unix_ms as u64).saturating_sub(horizon as u64) <= proto::GPS_HIST_RETENTION_MS
}

pub(crate) fn detection_fix_for_observation(
    gps_hist: &VecDeque<(i64, gps::Fix)>,
    gps_fix: &Arc<Mutex<Option<gps::Fix>>>,
    obs_unix: Option<i64>,
) -> Option<gps::Fix> {
    let fix_hist = obs_unix.and_then(|t| {
        choose_fix_from_history(gps_hist, t, MAX_FIX_TIMELINE_DELTA_MS)
            .filter(|f| wifi_scan::is_fix_usable_for_location(f))
    });
    let fix_snapshot = gps_fix.lock().ok().and_then(|g| g.clone());
    (fix_hist.or(fix_snapshot)).filter(|f| wifi_scan::is_fix_usable_for_location(f))
}
