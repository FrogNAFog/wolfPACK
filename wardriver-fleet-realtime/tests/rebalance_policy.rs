//! Host coverage for dedicated channel-plan rebalance decisions.

use wardriver_fleet_realtime::rebalance_policy::should_rebalance;

const MIN_INTERVAL_MS: u64 = 5_000;

#[test]
fn unchanged_state_does_not_rebalance() {
    assert!(!should_rebalance(
        Some(3),
        Some(10),
        3,
        10,
        MIN_INTERVAL_MS,
        MIN_INTERVAL_MS,
    ));
}

#[test]
fn plan_change_rebalances_without_waiting() {
    assert!(should_rebalance(
        Some(3),
        Some(10),
        4,
        10,
        1,
        MIN_INTERVAL_MS
    ));
}

#[test]
fn active_change_waits_for_minimum_interval() {
    assert!(!should_rebalance(
        Some(3),
        Some(10),
        3,
        11,
        MIN_INTERVAL_MS - 1,
        MIN_INTERVAL_MS,
    ));
    assert!(should_rebalance(
        Some(3),
        Some(10),
        3,
        11,
        MIN_INTERVAL_MS,
        MIN_INTERVAL_MS,
    ));
}

#[test]
fn deferred_active_change_remains_eligible() {
    let last_revision = Some(3);
    let last_active_sig = Some(10);

    assert!(!should_rebalance(
        last_revision,
        last_active_sig,
        3,
        11,
        4_999,
        MIN_INTERVAL_MS,
    ));
    assert!(should_rebalance(
        last_revision,
        last_active_sig,
        3,
        11,
        5_001,
        MIN_INTERVAL_MS,
    ));
}
