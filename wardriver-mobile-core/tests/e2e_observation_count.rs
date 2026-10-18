//! E2E: mobile enrichment observation row counts vs control pipeline semantics.
//!
//! Row count (not unique MAC count) is the parity metric: revisiting the same BSSID
//! beyond 50 m appends another observation row, matching control `MacTable` + WiGLE CSV.

use std::collections::VecDeque;

use wardriver_core::gps_anchor::GpsFix;
use wardriver_core::ieee80211_parse::try_ap_from_mgmt_mpdu;
use wardriver_mobile_core::enrichment::{
    enrich_wifi_ap_with_rcois, EnrichOutcome, EnrichmentState,
};
use wardriver_proto::{ApEntryV1, AuthMini};

fn gps_at(lat: f64, lon: f64, unix: i64) -> VecDeque<(i64, GpsFix)> {
    let mut h = VecDeque::new();
    h.push_back((
        unix,
        GpsFix {
            utc_unix_ms: unix,
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

fn ap(id: u8) -> ApEntryV1 {
    let mut ssid = [0u8; 32];
    ssid[..3].copy_from_slice(b"Net");
    ApEntryV1 {
        bssid: [0x02, 0x00, 0x00, 0x00, 0x00, id],
        channel: 6,
        rssi: -50,
        auth: AuthMini::Wpa2,
        ssid_len: 3,
        ssid,
        obs_mono_ms: 50_000,
    }
}

#[test]
fn observation_row_count_exceeds_unique_mac_on_revisit() {
    let unix = 1_700_000_000_000i64;
    let end_ms = 50_000u32;
    let mut state = EnrichmentState::default();
    let mac_ap = ap(1);

    state.gps_hist = gps_at(37.7749, -122.4194, unix);
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, end_ms, unix as u64, 3, &mac_ap, ""),
        EnrichOutcome::Stored(_)
    ));

    state.gps_hist = gps_at(37.7756, -122.4194, unix);
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, end_ms, unix as u64, 3, &mac_ap, ""),
        EnrichOutcome::Stored(_)
    ));

    // Two observation rows, one BSSID — matches control WiGLE row semantics.
    assert_eq!(state.next_id - 1, 2);
}

/// Golden scenario: two distinct BSSIDs + one beyond-radius revisit on the first MAC.
/// Expected rows: 3 (not 2 unique MACs).
#[test]
fn golden_row_count_two_macs_one_revisit() {
    const SCAN_END_MS: u32 = 50_000;
    const SCAN_END_UNIX_MS: u64 = 1_777_190_400_000;
    let unix = SCAN_END_UNIX_MS as i64;
    let mut state = EnrichmentState::default();
    state.gps_hist = gps_at(37.7749, -122.4194, unix);

    // AP A: ch6 fixture MAC (parsed from beacon bytes).
    let mut ap_a = try_ap_from_mgmt_mpdu(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        -45,
        6,
    )
    .expect("beacon parse");
    ap_a.obs_mono_ms = SCAN_END_MS;

    // AP B: ch36 fixture MAC.
    let mut ap_b = try_ap_from_mgmt_mpdu(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        -62,
        36,
    )
    .expect("beacon parse");
    ap_b.obs_mono_ms = SCAN_END_MS;

    // Row 1: first sighting of AP A.
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, SCAN_END_MS, SCAN_END_UNIX_MS, 3, &ap_a, ""),
        EnrichOutcome::Stored(_)
    ));
    // Row 2: first sighting of AP B (different BSSID).
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, SCAN_END_MS, SCAN_END_UNIX_MS, 3, &ap_b, ""),
        EnrichOutcome::Stored(_)
    ));
    // ~80 m north — beyond 50 m revisit on AP A → row 3 for same BSSID.
    state.gps_hist = gps_at(37.7756, -122.4194, unix);
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, SCAN_END_MS, SCAN_END_UNIX_MS, 3, &ap_a, ""),
        EnrichOutcome::Stored(_)
    ));

    // Three observation rows: two MACs + one revisit (control WiGLE semantics).
    assert_eq!(state.next_id - 1, 3);
}

#[test]
fn denied_observations_do_not_increment_row_count() {
    let unix = 1_700_000_000_000i64;
    let mut state = EnrichmentState::default();
    state.gps_hist = gps_at(37.77, -122.42, unix);
    state.inside_home_geofence = true;
    match enrich_wifi_ap_with_rcois(&mut state, 1, 50_000, unix as u64, 3, &ap(2), "") {
        EnrichOutcome::Dropped("csv_blocked_home_zone") => {}
        other => panic!("expected blocked, got {other:?}"),
    }
    assert_eq!(state.next_id, 1);
}
