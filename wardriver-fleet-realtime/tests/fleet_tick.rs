use wardriver_fleet_realtime::fleet_commands::PERIODIC_PING_INTERVAL_MS;
use wardriver_fleet_realtime::fleet_tick::{
    periodic_tick_should_run, queue_hint_tick_should_run, FleetTickState,
};

#[test]
fn fleet_tick_state_default_is_zeroed() {
    assert_eq!(
        FleetTickState::default(),
        FleetTickState {
            last_periodic_ping_ms: 0,
            last_ping_time_seq_bump_ms: 0,
        }
    );
}

#[test]
fn periodic_tick_requires_broadcast_ready() {
    let last = 1_000u64;
    let due_at = last + PERIODIC_PING_INTERVAL_MS;
    assert!(!periodic_tick_should_run(false, last, due_at));
    assert!(periodic_tick_should_run(true, last, due_at));
    assert!(!periodic_tick_should_run(true, last, due_at - 1));
}

#[test]
fn gateway_fleet_ping_broadcast_ready_helper_for_control_gating() {
    use wardriver_fleet_realtime::fleet_tick::fleet_ping_broadcast_ready;
    // Gateway fleet_loop: broadcast_ready = fleet_ping_broadcast_ready(policy_applied).
    // PackLink phone_connected must never gate this helper.
    assert!(!fleet_ping_broadcast_ready(false));
    assert!(fleet_ping_broadcast_ready(true));
    let last = 1_000u64;
    let due_at = last + PERIODIC_PING_INTERVAL_MS;
    assert!(!periodic_tick_should_run(
        fleet_ping_broadcast_ready(false),
        last,
        due_at
    ));
    assert!(periodic_tick_should_run(
        fleet_ping_broadcast_ready(true),
        last,
        due_at
    ));
}

#[test]
fn queue_hint_tick_requires_broadcast_ready_and_coalescing() {
    let last = 1_000u64;
    // Queue hint suppressed when regular ping is due within 50 ms.
    assert!(!queue_hint_tick_should_run(true, true, last, last + 250));
    assert!(queue_hint_tick_should_run(true, true, last, last + 249));
    // No work when gateway is not broadcast-ready.
    assert!(!queue_hint_tick_should_run(false, true, last, last + 249));
    // No work when peer pool did not flag a hint.
    assert!(!queue_hint_tick_should_run(true, false, last, last + 249));
}
