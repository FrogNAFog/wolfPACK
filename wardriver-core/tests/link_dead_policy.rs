//! Sniffer link-dead watchdog timing (mirrors [`wardriver_sniffer::sniffer_link`] elapsed math).

use wardriver_core::liveness::{
    link_dead_due, link_dead_elapsed_ms, link_reboot_due, SNIFFER_LINK_REBOOT_MS,
    SNIFFER_UPLINK_LIVENESS_MS,
};

fn arm_at(now_ms: u64) -> Option<u64> {
    Some(now_ms)
}

#[test]
fn rx_unchanged_19s_not_link_dead() {
    let armed = 10_000u64;
    let now = armed + SNIFFER_UPLINK_LIVENESS_MS - 1;
    assert_eq!(link_dead_elapsed_ms(arm_at(armed), now), Some(19_999));
    assert!(!link_dead_due(arm_at(armed), now));
    assert!(!link_reboot_due(arm_at(armed), now));
}

#[test]
fn rx_unchanged_20s_link_dead() {
    let armed = 10_000u64;
    let now = armed + SNIFFER_UPLINK_LIVENESS_MS;
    assert!(link_dead_due(arm_at(armed), now));
    assert!(!link_reboot_due(arm_at(armed), now));
}

#[test]
fn rx_unchanged_40s_reboot() {
    let armed = 0u64;
    let now = SNIFFER_LINK_REBOOT_MS;
    assert!(link_reboot_due(arm_at(armed), now));
}

#[test]
fn rx_advances_clears_armed_timer() {
    assert!(!link_dead_due(None, 50_000));
    assert_eq!(link_dead_elapsed_ms(None, 50_000), None);
}
