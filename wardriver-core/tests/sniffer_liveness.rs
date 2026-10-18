//! Host tests for sniffer control-liveness math (`wardriver_core::sniffer_liveness`).
//!
//! Covers all four (Some/None) combinations of the ESP-NOW RX and HTTP accept-contact timestamps.

use wardriver_core::sniffer_liveness::effective_control_liveness_ms;

#[test]
fn both_none_is_none() {
    assert_eq!(effective_control_liveness_ms(None, None), None);
}

#[test]
fn rx_only_returns_rx() {
    assert_eq!(
        effective_control_liveness_ms(Some(1_000), None),
        Some(1_000)
    );
}

#[test]
fn contact_only_returns_contact() {
    assert_eq!(
        effective_control_liveness_ms(None, Some(2_000)),
        Some(2_000)
    );
}

#[test]
fn both_some_returns_max() {
    // RX more recent.
    assert_eq!(
        effective_control_liveness_ms(Some(5_000), Some(3_000)),
        Some(5_000)
    );
    // Contact more recent.
    assert_eq!(
        effective_control_liveness_ms(Some(3_000), Some(5_000)),
        Some(5_000)
    );
    // Equal.
    assert_eq!(
        effective_control_liveness_ms(Some(4_000), Some(4_000)),
        Some(4_000)
    );
}
