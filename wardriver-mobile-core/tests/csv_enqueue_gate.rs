//! CSV enqueue sub-gate drop reasons.

use wardriver_mobile_core::enrichment::{
    csv_enqueue_allowed, csv_enqueue_block_reason, EnrichmentState,
};

#[test]
fn csv_enqueue_allowed_when_all_gates_pass() {
    let state = EnrichmentState::default();
    assert!(csv_enqueue_allowed(&state));
    assert_eq!(csv_enqueue_block_reason(&state), None);
}

#[test]
fn csv_blocked_untrusted_clock() {
    let mut state = EnrichmentState::default();
    state.trusted_clock = false;
    assert_eq!(
        csv_enqueue_block_reason(&state),
        Some("csv_blocked_untrusted_clock")
    );
}

#[test]
fn csv_blocked_scan_held() {
    let mut state = EnrichmentState::default();
    state.scan_held = true;
    assert_eq!(
        csv_enqueue_block_reason(&state),
        Some("csv_blocked_scan_held")
    );
}

#[test]
fn csv_blocked_home_zone() {
    let mut state = EnrichmentState::default();
    state.inside_home_geofence = true;
    assert_eq!(
        csv_enqueue_block_reason(&state),
        Some("csv_blocked_home_zone")
    );
}

#[test]
fn csv_blocked_gps_unusable() {
    let mut state = EnrichmentState::default();
    state.gps_usable = false;
    assert_eq!(
        csv_enqueue_block_reason(&state),
        Some("csv_blocked_gps_unusable")
    );
}

#[test]
fn csv_gate_priority_untrusted_clock_before_home_zone() {
    let mut state = EnrichmentState::default();
    state.trusted_clock = false;
    state.inside_home_geofence = true;
    assert_eq!(
        csv_enqueue_block_reason(&state),
        Some("csv_blocked_untrusted_clock")
    );
}
