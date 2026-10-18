//! Host tests for sniffer display helpers (`wardriver_core::liveness`).

use wardriver_core::liveness::{
    format_sniffer_last_seen_ago, pending_nearby_stale, pending_nearby_visible, short_sniffer_id,
    sniffer_is_quiet_stale, state_label, SNIFFER_QUIET_STALE_MS,
};

#[test]
fn pending_nearby_visible_boundary() {
    let last_seen = 10_000u64;
    assert!(pending_nearby_visible(29_999, last_seen, true));
    assert!(!pending_nearby_visible(30_000, last_seen, true));
    assert!(!pending_nearby_visible(29_999, last_seen, false));
}

#[test]
fn sniffer_is_quiet_stale_at_threshold() {
    assert!(sniffer_is_quiet_stale(30_000, 10_000, true));
    assert!(!sniffer_is_quiet_stale(29_999, 10_000, true));
}

#[test]
fn pending_nearby_stale_when_old() {
    assert!(pending_nearby_stale(50_000, 20_000, true));
}

#[test]
fn format_last_seen_ago() {
    assert_eq!(
        format_sniffer_last_seen_ago(20_000, Some(19_000)),
        "just now"
    );
    assert_eq!(format_sniffer_last_seen_ago(25_000, Some(20_000)), "5s ago");
    assert_eq!(format_sniffer_last_seen_ago(100_000, None), "-");
}

#[test]
fn state_label_wire_values() {
    assert_eq!(state_label(Some(1)), "SCANNING");
    assert_eq!(state_label(Some(0)), "IDLE");
    assert_eq!(state_label(None), "-");
    assert_eq!(state_label(Some(9)), "ERROR");
}

#[test]
fn short_sniffer_id_last_four_hex() {
    assert_eq!(short_sniffer_id(5), "...0005");
    assert_eq!(short_sniffer_id(0xDEAD_BEEF), "...BEEF");
}

#[test]
fn quiet_stale_matches_liveness_ms() {
    assert_eq!(SNIFFER_QUIET_STALE_MS, 20_000);
}
