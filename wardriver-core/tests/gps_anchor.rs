//! Host tests for GPS anchor math and timeline matching (`wardriver_core::gps_anchor`).
//!
//! Control CSV ingest drops rows when these helpers return `None` or when fixes fail
//! [`is_fix_usable_for_location`]; pipeline tests only cover the happy path.

use std::collections::VecDeque;

use wardriver_core::gps_anchor::{
    self, is_fix_usable_for_location, match_gps_fix_hist_index, observed_unix_ms_from_anchor,
    utc_string_from_unix_ms, GpsFix, MAX_FIX_TIMELINE_DELTA_MS,
};

/// Usable fix with sane defaults for timeline tests.
fn sample_fix() -> GpsFix {
    GpsFix {
        utc_unix_ms: 1_000_000,
        lat: 37.7749,
        lon: -122.4194,
        altitude_m: Some(12.0),
        hdop: Some(1.5),
        accuracy_m: None,
        fix_quality: 1,
    }
}

/// Build a sorted GPS history deque from `(unix_ms, fix)` pairs.
fn hist_with_samples(samples: &[(i64, GpsFix)]) -> VecDeque<(i64, GpsFix)> {
    samples.iter().cloned().collect()
}

#[test]
fn hdop_to_accuracy_m_floors_at_half() {
    // Values below 0.5 HDOP still map to at least 2.5 m accuracy.
    assert!((gps_anchor::hdop_to_accuracy_m(0.1) - 2.5).abs() < f32::EPSILON);
    assert!((gps_anchor::hdop_to_accuracy_m(1.0) - 5.0).abs() < f32::EPSILON);
}

#[test]
fn is_fix_usable_rejects_bad_quality() {
    let mut fix = sample_fix();
    fix.fix_quality = 0;
    assert!(!gps_anchor::is_fix_usable_for_location(&fix));
}

#[test]
fn is_fix_usable_rejects_missing_or_bad_hdop() {
    let mut fix = sample_fix();

    fix.hdop = None;
    assert!(!gps_anchor::is_fix_usable_for_location(&fix));

    fix.hdop = Some(f32::NAN);
    assert!(!gps_anchor::is_fix_usable_for_location(&fix));

    fix.hdop = Some(0.0);
    assert!(!gps_anchor::is_fix_usable_for_location(&fix));

    fix.hdop = Some(6.0);
    assert!(!gps_anchor::is_fix_usable_for_location(&fix));

    fix.hdop = Some(1.5);
    assert!(gps_anchor::is_fix_usable_for_location(&fix));
}

#[test]
fn observed_unix_ms_zero_anchor_returns_none() {
    // Sniffer has not applied wall clock yet — control must not emit UTC rows.
    assert!(gps_anchor::observed_unix_ms_from_anchor(30_000, 0, 29_000).is_none());
}

#[test]
fn observed_unix_ms_before_and_after_end_mono() {
    let end_mono = 30_000u32;
    let end_unix = 1_777_188_600_000u64;

    // Observation 1 s before scan end → Unix ms is 1000 ms earlier.
    let before =
        gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, 29_000).expect("before end");
    assert_eq!(before, 1_777_188_599_000);

    // Observation at scan end → anchor Unix unchanged.
    let at_end =
        gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, end_mono).expect("at end");
    assert_eq!(at_end, end_unix as i64);
}

#[test]
fn match_gps_empty_hist() {
    let hist = VecDeque::new();
    assert!(
        gps_anchor::match_gps_fix_for_obs(&hist, 1_000_000, MAX_FIX_TIMELINE_DELTA_MS).is_none()
    );
}

#[test]
fn match_gps_nearest_of_two() {
    let t0 = 1_000_000i64;
    let mut fix_a = sample_fix();
    fix_a.utc_unix_ms = t0;
    let mut fix_b = sample_fix();
    fix_b.utc_unix_ms = t0 + 1_500;

    let hist = hist_with_samples(&[(t0, fix_a), (t0 + 1_500, fix_b)]);
    // Observed 400 ms after T — closer to T (400 ms) than to T+1500 (1100 ms).
    let matched = gps_anchor::match_gps_fix_for_obs(&hist, t0 + 400, MAX_FIX_TIMELINE_DELTA_MS)
        .expect("match");
    assert_eq!(matched.utc_unix_ms, t0);
}

#[test]
fn match_gps_outside_max_delta() {
    let t0 = 1_000_000i64;
    let mut fix = sample_fix();
    fix.utc_unix_ms = t0;
    let hist = hist_with_samples(&[(t0, fix)]);

    // Nearest sample is 3000 ms away; default max delta is 2000 ms.
    assert!(
        gps_anchor::match_gps_fix_for_obs(&hist, t0 + 3_000, MAX_FIX_TIMELINE_DELTA_MS).is_none()
    );
}

#[test]
fn match_gps_unusable_nearest_fix() {
    let t0 = 1_000_000i64;

    let mut no_hdop = sample_fix();
    no_hdop.utc_unix_ms = t0;
    no_hdop.hdop = None;
    let hist_no_hdop = hist_with_samples(&[(t0, no_hdop)]);
    assert!(
        gps_anchor::match_gps_fix_for_obs(&hist_no_hdop, t0, MAX_FIX_TIMELINE_DELTA_MS).is_none()
    );

    let mut bad_quality = sample_fix();
    bad_quality.utc_unix_ms = t0;
    bad_quality.fix_quality = 0;
    let hist_bad_q = hist_with_samples(&[(t0, bad_quality)]);
    assert!(
        gps_anchor::match_gps_fix_for_obs(&hist_bad_q, t0, MAX_FIX_TIMELINE_DELTA_MS).is_none()
    );
}

#[test]
fn match_gps_hist_index_agrees_with_fix_ref() {
    let t0 = 1_000_000i64;
    let mut fix = sample_fix();
    fix.utc_unix_ms = t0;
    let hist = hist_with_samples(&[(t0, fix)]);

    let idx =
        gps_anchor::match_gps_fix_hist_index(&hist, t0, MAX_FIX_TIMELINE_DELTA_MS).expect("index");
    let fix_ref =
        gps_anchor::match_gps_fix_for_obs(&hist, t0, MAX_FIX_TIMELINE_DELTA_MS).expect("fix ref");
    assert_eq!(hist[idx].1, *fix_ref);
}

#[test]
fn utc_string_known_timestamps() {
    let cases: &[(i64, &str)] = &[
        (0, "1970-01-01 00:00:00"),
        (1_777_188_600_000, "2026-04-26 07:30:00"),
    ];
    for (unix_ms, expected) in cases {
        let got = gps_anchor::utc_string_from_unix_ms(*unix_ms).expect("utc string");
        assert_eq!(got, *expected, "unix_ms={unix_ms}");
    }
}

fn make_fix(utc_unix_ms: i64, hdop: Option<f32>, quality: u8) -> GpsFix {
    GpsFix {
        utc_unix_ms,
        lat: 37.0,
        lon: -122.0,
        altitude_m: None,
        hdop,
        accuracy_m: None,
        fix_quality: quality,
    }
}

fn make_full_fix(utc_unix_ms: i64) -> GpsFix {
    make_fix(utc_unix_ms, Some(1.0), 1)
}

#[test]
fn hdop_at_exact_boundary_5_0_is_usable() {
    let fix = make_fix(1_000_000, Some(5.0), 1);
    assert!(is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_just_above_5_0_is_rejected() {
    let fix = make_fix(1_000_000, Some(5.0001), 1);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_at_0_0_is_rejected() {
    let fix = make_fix(1_000_000, Some(0.0), 1);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_at_0_001_is_usable() {
    let fix = make_fix(1_000_000, Some(0.001), 1);
    assert!(is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_infinity_is_rejected() {
    let fix = make_fix(1_000_000, Some(f32::INFINITY), 1);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_nan_is_rejected() {
    let fix = make_fix(1_000_000, Some(f32::NAN), 1);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn fix_quality_zero_is_rejected() {
    let fix = make_fix(1_000_000, Some(1.0), 0);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_negative_is_rejected() {
    let fix = make_fix(1_000_000, Some(-1.0), 1);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn hdop_none_is_rejected() {
    let fix = make_fix(1_000_000, None, 1);
    assert!(!is_fix_usable_for_location(&fix));
}

#[test]
fn observed_unix_ms_with_no_anchor_returns_none() {
    let result = observed_unix_ms_from_anchor(1000, 0, 900);
    assert!(result.is_none());
}

#[test]
fn observed_unix_ms_before_anchor() {
    let result = observed_unix_ms_from_anchor(1000, 2_000_000, 900);
    assert_eq!(result, Some(1_999_900));
}

#[test]
fn observed_unix_ms_after_anchor() {
    let result = observed_unix_ms_from_anchor(1000, 2_000_000, 1100);
    assert_eq!(result, Some(2_000_100));
}

#[test]
fn observed_unix_ms_at_anchor() {
    let result = observed_unix_ms_from_anchor(1000, 2_000_000, 1000);
    assert_eq!(result, Some(2_000_000));
}

#[test]
fn match_rejects_future_timestamp_beyond_max_delta() {
    let fix = make_full_fix(1_000_000);
    let mut hist = VecDeque::new();
    hist.push_back((1_000_000, fix));
    let delta_plus_1 = (MAX_FIX_TIMELINE_DELTA_MS + 1) as i64;
    let result =
        match_gps_fix_hist_index(&hist, 1_000_000 + delta_plus_1, MAX_FIX_TIMELINE_DELTA_MS);
    assert!(result.is_none());
}

#[test]
fn match_accepts_timestamp_at_exact_max_delta() {
    let fix = make_full_fix(1_000_000);
    let mut hist = VecDeque::new();
    hist.push_back((1_000_000, fix));
    let delta = MAX_FIX_TIMELINE_DELTA_MS as i64;
    let result = match_gps_fix_hist_index(&hist, 1_000_000 + delta, MAX_FIX_TIMELINE_DELTA_MS);
    assert!(result.is_some());
}

#[test]
fn utc_string_at_epoch() {
    let s = utc_string_from_unix_ms(0);
    assert_eq!(s, Some("1970-01-01 00:00:00".to_string()));
}

#[test]
fn utc_string_with_negative_timestamp() {
    let s = utc_string_from_unix_ms(-86_400_000);
    assert_eq!(s, Some("1969-12-31 00:00:00".to_string()));
}

#[test]
fn utc_string_at_future_timestamp() {
    let s = utc_string_from_unix_ms(4_102_444_800_000);
    assert_eq!(s, Some("2100-01-01 00:00:00".to_string()));
}

#[test]
fn utc_string_with_subsecond_precision() {
    let s = utc_string_from_unix_ms(1_000_500);
    assert_eq!(s, Some("1970-01-01 00:16:40".to_string()));
}

#[test]
fn match_rejects_fix_with_bad_hdop() {
    let fix = make_fix(1_000_000, Some(10.0), 1);
    let mut hist = VecDeque::new();
    hist.push_back((1_000_000, fix));
    let result = match_gps_fix_hist_index(&hist, 1_000_000, MAX_FIX_TIMELINE_DELTA_MS);
    assert!(result.is_none());
}

#[test]
fn match_rejects_fix_with_bad_quality() {
    let fix = make_fix(1_000_000, Some(1.0), 0);
    let mut hist = VecDeque::new();
    hist.push_back((1_000_000, fix));
    let result = match_gps_fix_hist_index(&hist, 1_000_000, MAX_FIX_TIMELINE_DELTA_MS);
    assert!(result.is_none());
}

#[test]
fn match_on_empty_history() {
    let hist: VecDeque<(i64, GpsFix)> = VecDeque::new();
    let result = match_gps_fix_hist_index(&hist, 1_000_000, MAX_FIX_TIMELINE_DELTA_MS);
    assert!(result.is_none());
}
