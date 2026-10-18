//! GPS timeline matching and monotonic→UTC anchoring for WiGLE row encoding.
//!
//! Host-testable: no ESP-IDF types. Control firmware wraps these helpers and adds
//! stats counters where ingest drops rows.

use std::collections::VecDeque;

/// Minimum fix quality required to record a location on WiGLE rows.
pub const MIN_FIX_QUALITY: u8 = 1;

/// Maximum HDOP allowed to record a location (lower is better).
pub const MAX_HDOP: f32 = 5.0;

/// Max |observed_unix_ms − sample_unix_ms| to accept the nearest GPS history entry.
pub const MAX_FIX_TIMELINE_DELTA_MS: u64 = 2000;

/// GPS fix fields needed for WiGLE CSV row encoding (subset of control `gps::Fix`).
#[derive(Clone, Debug, PartialEq)]
pub struct GpsFix {
    pub utc_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<f32>,
    /// Control ESP32: HDOP from the GPS module. Mobile leaves this `None`.
    pub hdop: Option<f32>,
    /// Android fused location horizontal accuracy (m). Control leaves this `None`.
    pub accuracy_m: Option<f32>,
    pub fix_quality: u8,
}

/// Crude HDOP→accuracy mapping used by WiGLE `AccuracyMeters` column.
pub fn hdop_to_accuracy_m(hdop: f32) -> f32 {
    (hdop.max(0.5)) * 5.0
}

/// True when the fix is good enough to emit WiGLE rows with lat/lon and HDOP-derived accuracy.
pub fn is_fix_usable_for_location(f: &GpsFix) -> bool {
    if f.fix_quality < MIN_FIX_QUALITY {
        return false;
    }
    let Some(hdop) = f.hdop else {
        return false;
    };
    if !hdop.is_finite() || hdop <= 0.0 {
        return false;
    }
    hdop <= MAX_HDOP
}

/// Map sniffer monotonic observation time to Unix ms using the frag header anchor.
///
/// `end_mono` / `end_unix_ms` come from the scan batch header; `obs_mono_ms` is per-row.
pub fn observed_unix_ms_from_anchor(
    end_mono: u32,
    end_unix_ms: u64,
    obs_mono_ms: u32,
) -> Option<i64> {
    if end_unix_ms == 0 {
        return None;
    }
    Some(end_unix_ms as i64 + (obs_mono_ms as i64 - end_mono as i64))
}

fn abs_diff_ms_i64(a: i64, b: i64) -> u64 {
    a.saturating_sub(b).abs() as u64
}

/// Nearest `gps_hist` sample to `observed_unix_ms` (deque sorted by sample time).
pub fn best_gps_hist_match(
    hist: &VecDeque<(i64, GpsFix)>,
    observed_unix_ms: i64,
) -> Option<(u64, usize)> {
    if hist.is_empty() {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = hist.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if hist[mid].0 < observed_unix_ms {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let mut best: Option<(u64, usize)> = None;
    for idx in [lo.saturating_sub(1), lo] {
        if idx < hist.len() {
            let d = abs_diff_ms_i64(hist[idx].0, observed_unix_ms);
            if best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
                best = Some((d, idx));
            }
        }
    }
    best
}

/// Nearest history sample within `max_delta_ms` that passes [`is_fix_usable_for_location`].
pub fn match_gps_fix_for_obs<'a>(
    hist: &'a VecDeque<(i64, GpsFix)>,
    observed_unix_ms: i64,
    max_delta_ms: u64,
) -> Option<&'a GpsFix> {
    let idx = match_gps_fix_hist_index(hist, observed_unix_ms, max_delta_ms)?;
    Some(&hist.get(idx)?.1)
}

/// Index into `hist` for the best GPS match, or `None` when no usable sample is in range.
pub fn match_gps_fix_hist_index(
    hist: &VecDeque<(i64, GpsFix)>,
    observed_unix_ms: i64,
    max_delta_ms: u64,
) -> Option<usize> {
    let (d, idx) = best_gps_hist_match(hist, observed_unix_ms)?;
    if d > max_delta_ms {
        return None;
    }
    let fix = &hist.get(idx)?.1;
    if !is_fix_usable_for_location(fix) {
        return None;
    }
    Some(idx)
}

/// Format Unix milliseconds as `YYYY-MM-DD hh:mm:ss` UTC (WiGLE `FirstSeen` column).
pub fn utc_string_from_unix_ms(unix_ms: i64) -> Option<String> {
    let t = unix_ms.div_euclid(1000);
    let days = t.div_euclid(86_400);
    let tod = t.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = (tod / 3600) as u32;
    let mi = ((tod % 3600) / 60) as u32;
    let s = (tod % 60) as u32;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, mo, d, h, mi, s
    ))
}

/// Proleptic Gregorian calendar from Unix day number (algorithm from Howard Hinnant).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mp >= 10 { y + 1 } else { y };
    (y, m, d)
}
