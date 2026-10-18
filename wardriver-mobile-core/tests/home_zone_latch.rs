//! Host tests for home-zone latch (`wardriver_mobile_core::home_zone_latch`).

use wardriver_mobile_core::home_zone_latch::{HomeZoneLatch, HOME_ZONE_EXIT_SUSTAINED_MS};

const CENTER_LAT: f64 = 37.7749;
const CENTER_LON: f64 = -122.4194;
const RADIUS_M: f64 = 100.0;

#[test]
fn disabled_resets_and_returns_false() {
    let mut latch = HomeZoneLatch::new();
    latch.inside_prev = true;
    latch.outside_gps_ok_since_ms = 999;
    assert!(!latch.wardrive_suppressed(
        false,
        CENTER_LAT,
        CENTER_LON,
        RADIUS_M,
        Some(CENTER_LAT),
        Some(CENTER_LON),
        true,
        1000
    ));
    assert!(!latch.inside_prev);
    assert_eq!(latch.outside_gps_ok_since_ms, 0);
}

#[test]
fn enters_when_inside_radius() {
    let mut latch = HomeZoneLatch::new();
    assert!(latch.wardrive_suppressed(
        true,
        CENTER_LAT,
        CENTER_LON,
        RADIUS_M,
        Some(CENTER_LAT),
        Some(CENTER_LON),
        true,
        1000,
    ));
    assert!(latch.inside_prev);
}

#[test]
fn latched_inside_survives_gps_dropout() {
    let mut latch = HomeZoneLatch::new();
    latch.inside_prev = true;
    assert!(
        latch.wardrive_suppressed(true, CENTER_LAT, CENTER_LON, RADIUS_M, None, None, false, 2000)
    );
}

#[test]
fn sustained_exit_clears_latch() {
    let mut latch = HomeZoneLatch::new();
    latch.inside_prev = true;
    // Far outside (~1.1 km north)
    let far_lat = CENTER_LAT + 0.01;
    let t0 = 10_000u64;
    assert!(latch.wardrive_suppressed(
        true,
        CENTER_LAT,
        CENTER_LON,
        RADIUS_M,
        Some(far_lat),
        Some(CENTER_LON),
        true,
        t0
    ));
    assert!(latch.inside_prev);
    let t1 = t0 + HOME_ZONE_EXIT_SUSTAINED_MS;
    assert!(!latch.wardrive_suppressed(
        true,
        CENTER_LAT,
        CENTER_LON,
        RADIUS_M,
        Some(far_lat),
        Some(CENTER_LON),
        true,
        t1
    ));
    assert!(!latch.inside_prev);
}

#[test]
fn contains_point_in_circle() {
    assert!(HomeZoneLatch::contains(
        CENTER_LAT, CENTER_LON, RADIUS_M, CENTER_LAT, CENTER_LON
    ));
}
