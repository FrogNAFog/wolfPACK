//! Host tests for mobile (Android) GPS anchor helpers (`wardriver_mobile_core::gps_anchor_mobile`).

use std::collections::VecDeque;

use wardriver_core::gps_anchor::{GpsFix, MAX_FIX_TIMELINE_DELTA_MS};
use wardriver_mobile_core::gps_anchor_mobile::{
    is_fix_usable_for_mobile, match_mobile_gps_fix_for_obs, MAX_MOBILE_ACCURACY_M,
};

fn mobile_fix(accuracy_m: f32) -> GpsFix {
    GpsFix {
        utc_unix_ms: 1_000_000,
        lat: 37.7749,
        lon: -122.4194,
        altitude_m: Some(12.0),
        hdop: None,
        accuracy_m: Some(accuracy_m),
        fix_quality: 1,
    }
}

#[test]
fn mobile_usable_accepts_sub_two_point_five_meters() {
    // No HDOP floor — 1 m fused accuracy is valid on the phone path.
    assert!(is_fix_usable_for_mobile(&mobile_fix(1.0)));
}

#[test]
fn mobile_usable_rejects_zero_nan_and_above_ceiling() {
    assert!(!is_fix_usable_for_mobile(&mobile_fix(0.0)));
    let mut bad = mobile_fix(8.0);
    bad.accuracy_m = Some(f32::NAN);
    assert!(!is_fix_usable_for_mobile(&bad));
    assert!(!is_fix_usable_for_mobile(&mobile_fix(
        MAX_MOBILE_ACCURACY_M + 0.1
    )));
}

#[test]
fn mobile_usable_rejects_missing_accuracy() {
    let mut fix = mobile_fix(8.0);
    fix.accuracy_m = None;
    assert!(!is_fix_usable_for_mobile(&fix));
}

#[test]
fn mobile_match_finds_nearest_usable_sample() {
    let t0 = 1_000_000i64;
    let hist = VecDeque::from([(t0, mobile_fix(8.0)), (t0 + 1_500, mobile_fix(10.0))]);
    let matched =
        match_mobile_gps_fix_for_obs(&hist, t0 + 100, MAX_FIX_TIMELINE_DELTA_MS).expect("match");
    assert!((matched.accuracy_m.unwrap() - 8.0).abs() < f32::EPSILON);
}

#[test]
fn mobile_match_rejects_hdop_only_fix() {
    // Control-style fix without accuracy_m must not match on the mobile path.
    let hist = VecDeque::from([(
        1_000_000i64,
        GpsFix {
            utc_unix_ms: 1_000_000,
            lat: 37.7749,
            lon: -122.4194,
            altitude_m: Some(12.0),
            hdop: Some(1.5),
            accuracy_m: None,
            fix_quality: 1,
        },
    )]);
    assert!(match_mobile_gps_fix_for_obs(&hist, 1_000_000, MAX_FIX_TIMELINE_DELTA_MS).is_none());
}
