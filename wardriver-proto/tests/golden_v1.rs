//! Stable v1 wire bytes for regression (golden fixtures under `tests/golden/v1/`).

use wardriver_proto::{
    pack_control_cmd_vec, pack_ping_vec, pack_sniffer_ack, ControlCmd, ControlCmdV1, SnifferAckV1,
    SnifferRole, SnifferState, CHANNELS_MAX, PROTO_VERSION,
};

#[cfg(not(feature = "mgmt-ap-5ghz"))]
#[test]
fn golden_control_cmd_v1_bytes() {
    let wire = sample_control_cmd_wire();
    let golden = include_bytes!("golden/v1/control_cmd_v1.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "control_cmd_v1 wire changed; refresh tests/golden/v1/control_cmd_v1.bin"
    );
}

#[cfg(feature = "mgmt-ap-5ghz")]
#[test]
fn golden_control_cmd_v1_5ghz_bytes() {
    let wire = sample_control_cmd_wire();
    let golden = include_bytes!("golden/v1/control_cmd_v1_5ghz.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "control_cmd_v1_5ghz wire changed; refresh tests/golden/v1/control_cmd_v1_5ghz.bin"
    );
}

#[test]
fn golden_control_cmd_ping_v7_bytes() {
    let wire = sample_ping_cmd_wire();
    let golden = include_bytes!("golden/v1/control_cmd_ping_v7.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "control_cmd_ping_v7 wire changed; refresh tests/golden/v1/control_cmd_ping_v7.bin"
    );
}

#[test]
fn golden_sniffer_ack_v1_bytes() {
    let wire = sample_sniffer_ack_wire();
    let golden = include_bytes!("golden/v1/sniffer_ack_v1.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "sniffer_ack_v1 wire changed; refresh tests/golden/v1/sniffer_ack_v1.bin"
    );
}

fn sample_control_cmd_wire() -> Vec<u8> {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[..4].copy_from_slice(&[1, 6, 11, 36]);
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::SetChannelSchedule,
        cmd_seq: 42,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        dwell_ms: 200,
        chan_count: 4,
        channels,
        desired_role: 0,
        control_boot_id: 0xAABB_CCDD,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 0,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    pack_control_cmd_vec(cmd).expect("pack control cmd")
}

fn sample_ping_cmd_wire() -> Vec<u8> {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 42,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0xAABB_CCDD,
        detection_disable_wifi_mask: 0x0000_0001,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0x0000_0004,
        desired_scanning: 1,
        peer_slots_free: 4,
        peer_join_waiters: 2,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    pack_ping_vec(cmd).expect("pack ping")
}

fn sample_sniffer_ack_wire() -> Vec<u8> {
    let ack = SnifferAckV1 {
        sniffer_id: 7,
        ack_cmd_seq: 99,
        state: SnifferState::Scanning,
        role: SnifferRole::Wifi,
        ble_capable: 1,
        sniffer_boot_id: 0x1122_3344,
        free_heap_bytes: 120_000,
        total_heap_bytes: 200_000,
        cpu_load_pct: 37,
        five_ghz_capable: 1,
        dfs_capable: 0,
        bulk_lost_cycles: 0,
    };
    pack_sniffer_ack(ack)
}

/// Host-only: regenerate golden binaries after an intentional wire layout change.
#[test]
#[ignore]
fn write_golden_v1_fixtures() {
    std::fs::create_dir_all("tests/golden/v1").expect("mkdir golden/v1");
    std::fs::write(
        "tests/golden/v1/control_cmd_v1.bin",
        sample_control_cmd_wire(),
    )
    .expect("write control_cmd");
    std::fs::write(
        "tests/golden/v1/control_cmd_ping_v7.bin",
        sample_ping_cmd_wire(),
    )
    .expect("write control_cmd_ping");
    std::fs::write(
        "tests/golden/v1/sniffer_ack_v1.bin",
        sample_sniffer_ack_wire(),
    )
    .expect("write sniffer_ack");
    assert_eq!(sample_control_cmd_wire()[0], PROTO_VERSION);
}
