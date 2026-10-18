//! Mobile enrichment: multiple observation rows per MAC when moved >50 m.

use std::collections::VecDeque;

use wardriver_core::gps_anchor::GpsFix;
use wardriver_mobile_core::enrichment::{
    enrich_wifi_ap_with_rcois, EnrichOutcome, EnrichmentState, REVISIT_RADIUS_M,
};
use wardriver_proto::{ApEntryV1, AuthMini};

fn gps_hist_at(lat: f64, lon: f64, unix_ms: i64) -> VecDeque<(i64, GpsFix)> {
    let mut h = VecDeque::new();
    h.push_back((
        unix_ms,
        GpsFix {
            utc_unix_ms: unix_ms,
            lat,
            lon,
            altitude_m: Some(10.0),
            hdop: None,
            accuracy_m: Some(6.0),
            fix_quality: 1,
        },
    ));
    h
}

fn sample_ap(bssid: u8, rssi: i8) -> ApEntryV1 {
    let mut ssid = [0u8; 32];
    ssid[..4].copy_from_slice(b"Test");
    ApEntryV1 {
        bssid: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, bssid],
        channel: 6,
        rssi,
        auth: AuthMini::Wpa2,
        ssid_len: 4,
        ssid,
        obs_mono_ms: 50_000,
    }
}

#[test]
fn revisit_within_radius_drops_second_row() {
    let unix = 1_700_000_000_000i64;
    let mut state = EnrichmentState::default();
    state.gps_hist = gps_hist_at(37.7749, -122.4194, unix);
    let ap = sample_ap(1, -55);
    let end_ms = 50_000u32;
    match enrich_wifi_ap_with_rcois(&mut state, 1, end_ms, unix as u64, 3, &ap, "") {
        EnrichOutcome::Stored(_) => {}
        other => panic!("expected stored, got {other:?}"),
    }
    // ~30 m north — still within 50 m revisit
    state.gps_hist = gps_hist_at(37.77515, -122.4194, unix);
    match enrich_wifi_ap_with_rcois(&mut state, 1, end_ms, unix as u64, 3, &ap, "") {
        EnrichOutcome::Dropped("mac_table_dedup") => {}
        other => panic!("expected dedup drop, got {other:?}"),
    }
}

#[test]
fn revisit_beyond_radius_appends_second_row() {
    let unix = 1_700_000_000_000i64;
    let mut state = EnrichmentState::default();
    state.gps_hist = gps_hist_at(37.7749, -122.4194, unix);
    let ap = sample_ap(2, -60);
    let end_ms = 50_000u32;
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, end_ms, unix as u64, 3, &ap, ""),
        EnrichOutcome::Stored(_)
    ));
    // ~80 m north — beyond REVISIT_RADIUS_M
    assert!(REVISIT_RADIUS_M < 80.0);
    state.gps_hist = gps_hist_at(37.7756, -122.4194, unix);
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, end_ms, unix as u64, 3, &ap, ""),
        EnrichOutcome::Stored(_)
    ));
    assert_eq!(state.next_id, 3);
}

#[test]
fn geofence_leave_detected() {
    use wardriver_mobile_core::geofence::{GeofenceEvent, GeofenceMonitor, GeofenceSquare};

    let square = GeofenceSquare {
        lat_min: 37.77,
        lat_max: 37.78,
        lon_min: -122.42,
        lon_max: -122.41,
    };
    let mut mon = GeofenceMonitor::new(square, true);
    assert_eq!(mon.update(37.775, -122.415), GeofenceEvent::Entered);
    assert_eq!(mon.update(37.79, -122.415), GeofenceEvent::Left);
}

#[test]
fn coverage_grid_fraction() {
    use wardriver_mobile_core::coverage_grid::CoverageGrid;
    use wardriver_mobile_core::geofence::GeofenceSquare;

    let square = GeofenceSquare {
        lat_min: 0.0,
        lat_max: 1.0,
        lon_min: 0.0,
        lon_max: 1.0,
    };
    let mut grid = CoverageGrid::new(square, 2, 2);
    grid.visit(0.25, 0.25);
    assert!((grid.coverage_fraction() - 0.25).abs() < 0.01);
}
