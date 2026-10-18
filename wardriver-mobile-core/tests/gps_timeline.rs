//! Host tests for mobile GPS timeline (`wardriver_mobile_core::gps_timeline`).

use wardriver_mobile_core::gps_timeline::{GpsTimeline, GPS_HIST_RETENTION_MS, MAX_FUTURE_SKEW_MS};

#[test]
fn valid_fix_time_rejects_future_skew() {
    let now = 1_700_000_000_000i64;
    assert!(GpsTimeline::is_valid_fix_time(now, now));
    assert!(GpsTimeline::is_valid_fix_time(
        now + MAX_FUTURE_SKEW_MS,
        now
    ));
    assert!(!GpsTimeline::is_valid_fix_time(
        now + MAX_FUTURE_SKEW_MS + 1,
        now
    ));
    assert!(!GpsTimeline::is_valid_fix_time(0, now));
}

#[test]
fn append_overwrites_same_unix_ms() {
    let now = 1_700_000_000_000i64;
    let mut tl = GpsTimeline::new();
    assert!(tl.append(now, 37.77, -122.42, 8.0, None, now));
    assert!(tl.append(now, 37.78, -122.43, 9.0, Some(12.0), now));
    assert_eq!(tl.len(), 1);
    let hist = tl.to_fix_hist();
    assert!((hist[0].1.lat - 37.78).abs() < f64::EPSILON);
}

#[test]
fn prune_drops_old_samples() {
    let now = 1_700_000_000_000i64;
    let mut tl = GpsTimeline::new();
    tl.append(
        now - GPS_HIST_RETENTION_MS - 1_000,
        37.77,
        -122.42,
        8.0,
        None,
        now,
    );
    tl.append(now - 30_000, 37.77, -122.42, 8.0, None, now);
    assert_eq!(tl.len(), 1);
    assert_eq!(tl.to_fix_hist()[0].0, now - 30_000);
}

#[test]
fn stale_replay_boundary() {
    let now = 1_700_000_000_000i64;
    let anchor = now - GPS_HIST_RETENTION_MS - 1;
    assert!(GpsTimeline::is_stale_replay(anchor, now));
    assert!(!GpsTimeline::is_stale_replay(now - 30_000, now));
}

#[test]
fn is_usable_accuracy_ceiling() {
    assert!(GpsTimeline::is_usable(8.0));
    assert!(GpsTimeline::is_usable(25.0));
    assert!(!GpsTimeline::is_usable(0.0));
    assert!(!GpsTimeline::is_usable(26.0));
}

#[test]
fn merge_samples_from_route_points() {
    let now = 1_700_000_000_000i64;
    let mut tl = GpsTimeline::new();
    tl.merge_samples(
        [
            (now - 1_000, 37.77, -122.42, 10.0, None),
            (now, 37.78, -122.43, 8.0, None),
        ],
        now,
    );
    assert_eq!(tl.len(), 2);
}
