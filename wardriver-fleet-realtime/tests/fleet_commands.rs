use wardriver_fleet_realtime::fleet_commands::{
    build_start_stop, build_welcome, periodic_ping_due, queue_hint_ping_should_send, FleetList,
    StartStopConfig, WelcomeConfig, PERIODIC_PING_INTERVAL_MS,
};
use wardriver_proto::{unpack_ssid_watchlist, ControlCmd, CHANNELS_MAX, PROTO_VERSION};

#[test]
fn welcome_and_start_stop_clear_schedule_when_scanning_is_disabled() {
    let schedule = Some(([6u8; CHANNELS_MAX], 3));
    let welcome = build_welcome(WelcomeConfig {
        control_boot_id: 9,
        control_channel: 11,
        scan_allowed: false,
        dwell_ms: 120,
        schedule,
        detection_disable_wifi_mask: 1,
        wardrive_suppressed: true,
        fleet_slot: 4,
        detection_disable_ble_mask: 2,
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    });
    assert_eq!(welcome.proto_min, PROTO_VERSION);
    assert_eq!(welcome.proto_max, PROTO_VERSION);
    assert_eq!(welcome.desired_scanning, 0);
    assert_eq!(welcome.chan_count, 0);
    assert_eq!(welcome.channels, [0; CHANNELS_MAX]);

    let cmd = build_start_stop(StartStopConfig {
        cmd_seq: 7,
        control_boot_id: 9,
        control_channel: 11,
        scan_allowed: false,
        dwell_ms: 120,
        schedule,
        desired_role: 1,
        detection_disable_wifi_mask: 1,
        wardrive_suppressed: true,
        time_set_seq: 5,
        ctrl_unix_ms: 123,
        detection_disable_ble_mask: 2,
        peer_slots_free: 3,
        peer_join_waiters: 4,
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    });
    assert_eq!(cmd.cmd, ControlCmd::StopScanning);
    assert_eq!(cmd.chan_count, 0);
    assert_eq!(cmd.channels, [0; CHANNELS_MAX]);
    assert_eq!(cmd.desired_role, 1);
}

#[test]
fn fleet_list_preserves_empty_watchlist_clear() {
    let wire = FleetList::SsidWatchlist(&[]).pack().expect("pack");
    assert!(unpack_ssid_watchlist(&wire).expect("unpack").is_empty());
}

#[test]
fn ping_cadence_and_queue_hint_coalescing_match() {
    assert!(!periodic_ping_due(
        1_000,
        1_000 + PERIODIC_PING_INTERVAL_MS - 1
    ));
    assert!(periodic_ping_due(1_000, 1_000 + PERIODIC_PING_INTERVAL_MS));
    assert!(queue_hint_ping_should_send(1_000, 1_249));
    assert!(!queue_hint_ping_should_send(1_000, 1_250));
    assert!(queue_hint_ping_should_send(1_000, 1_300));
}
