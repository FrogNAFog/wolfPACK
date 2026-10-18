//! TimeSync ping/pong wire + RTT math helpers.

use wardriver_proto::{
    coarse_time_set_suppressed, mono_delta_ms, one_way_ms_from_rtt, pack_time_sync_ping,
    pack_time_sync_pong, rtt_sane, should_apply_coarse_time_set, time_set_unix_ms, time_skew_ms,
    unpack_time_sync_ping, unpack_time_sync_pong, wall_unix_plausible, TimeSyncPingV1,
    TimeSyncPongV1, TIME_SYNC_COARSE_SUPPRESS_MS, TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS,
    TIME_SYNC_SKEW_FORCE_MS,
};

#[test]
fn time_sync_ping_pong_roundtrip() {
    let ping = TimeSyncPingV1 {
        seq: 7,
        ctrl_tx_ms: 1_234_567,
    };
    let back = unpack_time_sync_ping(&pack_time_sync_ping(ping)).expect("ping unpack");
    assert_eq!(back, ping);

    let pong = TimeSyncPongV1 {
        seq: 7,
        sniff_rx_ms: 1_234_580,
        sniffer_id: 42,
    };
    let back = unpack_time_sync_pong(&pack_time_sync_pong(pong)).expect("pong unpack");
    assert_eq!(back, pong);
}

#[test]
fn mono_delta_ms_wraps() {
    assert_eq!(mono_delta_ms(0xFFFF_FFF0, 0x20), 48);
    assert_eq!(mono_delta_ms(100, 250), 150);
}

#[test]
fn time_set_unix_ms_golden() {
    assert_eq!(one_way_ms_from_rtt(20), 10);
    assert_eq!(time_set_unix_ms(1_700_000_000_000, 20), 1_700_000_000_010);
}

#[test]
fn rtt_sane_bounds() {
    assert!(!rtt_sane(0));
    assert!(rtt_sane(1));
    assert!(rtt_sane(500));
    assert!(!rtt_sane(501));
}

#[test]
fn coarse_time_set_suppressed_after_rtt() {
    let last_rtt = 1_000_000u64;
    assert!(!coarse_time_set_suppressed(0, last_rtt + 100));
    assert!(coarse_time_set_suppressed(last_rtt, last_rtt + 600_000));
    assert!(!coarse_time_set_suppressed(
        last_rtt,
        last_rtt + TIME_SYNC_COARSE_SUPPRESS_MS
    ));
}

#[test]
fn wall_unix_plausible_threshold() {
    assert!(!wall_unix_plausible(TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS - 1));
    assert!(wall_unix_plausible(TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS));
}

#[test]
fn should_apply_coarse_epoch_recovery() {
    let now = 5_000_000u64;
    let offered = 1_700_000_000_000u64;
    // Boot epoch local: always accept plausible offer even inside RTT suppress window.
    assert!(should_apply_coarse_time_set(now, now + 100, 0, offered));
}

#[test]
fn should_apply_coarse_skew_bypass() {
    let last_rtt = 1_000_000u64;
    let now = last_rtt + 60_000;
    let local = 1_700_000_000_000u64;
    let offered = local + TIME_SYNC_SKEW_FORCE_MS + 1;
    assert!(coarse_time_set_suppressed(last_rtt, now));
    assert!(should_apply_coarse_time_set(last_rtt, now, local, offered));
}

#[test]
fn should_apply_coarse_respects_suppress_when_in_sync() {
    let last_rtt = 1_000_000u64;
    let now = last_rtt + 60_000;
    let local = 1_700_000_000_000u64;
    let offered = local + 100;
    assert!(coarse_time_set_suppressed(last_rtt, now));
    assert!(!should_apply_coarse_time_set(last_rtt, now, local, offered));
}

#[test]
fn time_skew_ms_symmetric() {
    let a = 1_700_000_000_000u64;
    let b = a + 42;
    assert_eq!(time_skew_ms(a, b), 42);
    assert_eq!(time_skew_ms(b, a), 42);
}
