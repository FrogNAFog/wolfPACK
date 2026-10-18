use wardriver_core::boot_button_hold::{
    hold_complete, hold_seconds_remaining, FLEET_UNBIND_HOLD_MS,
};

#[test]
fn hold_complete_at_threshold() {
    assert!(!hold_complete(4_999, FLEET_UNBIND_HOLD_MS));
    assert!(hold_complete(5_000, FLEET_UNBIND_HOLD_MS));
    assert!(hold_complete(6_000, FLEET_UNBIND_HOLD_MS));
}

#[test]
fn hold_seconds_remaining_counts_down() {
    assert_eq!(hold_seconds_remaining(0, 5_000), Some(5));
    assert_eq!(hold_seconds_remaining(1_000, 5_000), Some(4));
    assert_eq!(hold_seconds_remaining(4_999, 5_000), Some(1));
    assert_eq!(hold_seconds_remaining(5_000, 5_000), None);
}
