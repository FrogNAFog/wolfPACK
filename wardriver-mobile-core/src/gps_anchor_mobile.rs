//! Mobile (Android) GPS timeline matching — native fused `accuracy_m`, not HDOP.
//!
//! Control firmware uses [`gps_anchor::is_fix_usable_for_location`] (HDOP-based).
//! The phone path carries Android `Location.getAccuracy()` through ingest unchanged.

use std::collections::VecDeque;

use crate::gps_anchor::{self, GpsFix, MIN_FIX_QUALITY};

/// Matches Android [`ObservationIngestor.ROUTE_MAX_ACCURACY_M`].
pub const MAX_MOBILE_ACCURACY_M: f32 = 25.0;

/// True when a phone GPS sample is good enough to anchor a WiGLE observation row.
///
/// Requires a finite native `accuracy_m` in `(0, MAX_MOBILE_ACCURACY_M]`.
/// No HDOP floor — sub-2.5 m fused fixes are accepted as-is.
#[must_use]
pub fn is_fix_usable_for_mobile(f: &GpsFix) -> bool {
    if f.fix_quality < MIN_FIX_QUALITY {
        return false;
    }
    let Some(acc) = f.accuracy_m else {
        return false;
    };
    acc.is_finite() && acc > 0.0 && acc <= MAX_MOBILE_ACCURACY_M
}

/// Nearest history sample within `max_delta_ms` that passes [`is_fix_usable_for_mobile`].
#[must_use]
pub fn match_mobile_gps_fix_for_obs<'a>(
    hist: &'a VecDeque<(i64, GpsFix)>,
    observed_unix_ms: i64,
    max_delta_ms: u64,
) -> Option<&'a GpsFix> {
    let idx = match_mobile_gps_fix_hist_index(hist, observed_unix_ms, max_delta_ms)?;
    Some(&hist.get(idx)?.1)
}

/// Index into `hist` for the best mobile GPS match, or `None` when out of range / unusable.
#[must_use]
pub fn match_mobile_gps_fix_hist_index(
    hist: &VecDeque<(i64, GpsFix)>,
    observed_unix_ms: i64,
    max_delta_ms: u64,
) -> Option<usize> {
    let (d, idx) = gps_anchor::best_gps_hist_match(hist, observed_unix_ms)?;
    if d > max_delta_ms {
        return None;
    }
    let fix = &hist.get(idx)?.1;
    if !is_fix_usable_for_mobile(fix) {
        return None;
    }
    Some(idx)
}
