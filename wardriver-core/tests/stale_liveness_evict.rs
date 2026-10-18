//! Stale uplink liveness eviction policy (mirrors control `tick_stale_liveness_evict`).

use wardriver_core::liveness::{stale_uplink_peer_should_evict, SNIFFER_UPLINK_LIVENESS_MS};

#[test]
fn fresh_last_seen_not_evicted() {
    let now = 50_000u64;
    let last_seen = now.saturating_sub(SNIFFER_UPLINK_LIVENESS_MS);
    assert!(!stale_uplink_peer_should_evict(last_seen, now));
}

#[test]
fn stale_last_seen_evicted() {
    let now = 50_000u64;
    let last_seen = now.saturating_sub(SNIFFER_UPLINK_LIVENESS_MS + 1);
    assert!(stale_uplink_peer_should_evict(last_seen, now));
}

#[test]
fn boundary_at_liveness_budget_not_evicted() {
    let now = 100_000u64;
    let last_seen = now.saturating_sub(SNIFFER_UPLINK_LIVENESS_MS);
    assert!(!stale_uplink_peer_should_evict(last_seen, now));
}

#[test]
fn zero_last_seen_evicted() {
    assert!(stale_uplink_peer_should_evict(0, 1));
}
