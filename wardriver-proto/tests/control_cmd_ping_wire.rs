//! PROTO v8 `ControlCmd::Ping` short wire (peer pool hints + desired_scanning).

use wardriver_proto::{
    pack_control_cmd_vec, pack_ping_vec, unpack_control_cmd, unpack_ping, ControlCmd, ControlCmdV1,
    CHANNELS_MAX, PROTO_VERSION,
};

// -- desired_scanning tests --

#[test]
fn control_cmd_ping_desired_scanning_roundtrip() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 100,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0xDEAD_BEEF,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 1,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let wire = pack_ping_vec(cmd).expect("pack ping");
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire.len(), 49);
    let back = unpack_ping(&wire).expect("unpack ping cmd");
    assert_eq!(back.desired_scanning, 1);
    assert_eq!(back.cmd, ControlCmd::Ping);
    assert_eq!(back.chan_count, 0);
}

#[test]
fn control_cmd_desired_scanning_clamped_on_wire() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 99,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let wire = pack_ping_vec(cmd).expect("pack ping");
    let back = unpack_ping(&wire).expect("unpack");
    assert_eq!(back.desired_scanning, 1);
}

#[test]
fn pack_control_cmd_vec_rejects_ping() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0,
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
    assert!(pack_control_cmd_vec(cmd).is_err());
}

#[test]
fn unpack_control_cmd_rejects_ping_on_full_body() {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[..3].copy_from_slice(&[1, 6, 11]);
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::SetChannelSchedule,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 200,
        chan_count: 3,
        channels,
        desired_role: 0,
        control_boot_id: 0,
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
    let mut wire = pack_control_cmd_vec(cmd).expect("pack schedule");
    wire[10] = ControlCmd::Ping as u8;
    assert!(unpack_control_cmd(&wire).is_err());
}

// -- peer_hints tests --

#[test]
fn control_cmd_ping_peer_hints_roundtrip() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 1,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 1,
        peer_slots_free: 4,
        peer_join_waiters: 2,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let wire = pack_ping_vec(cmd).expect("pack ping");
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire.len(), 49);
    let back = unpack_ping(&wire).expect("unpack");
    assert_eq!(back.peer_slots_free, 4);
    assert_eq!(back.peer_join_waiters, 2);
}

#[test]
fn control_cmd_peer_slots_free_clamped_on_wire() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 0,
        peer_slots_free: 99,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let wire = pack_ping_vec(cmd).expect("pack ping");
    let back = unpack_ping(&wire).expect("unpack");
    assert_eq!(back.peer_slots_free, 16);
}

#[test]
fn control_cmd_ping_short_form_rejected_when_truncated() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0,
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
    let mut wire = pack_ping_vec(cmd).expect("pack ping");
    wire.truncate(48); // missing trailing ble_active_scan
    assert!(unpack_ping(&wire).is_err());
}

#[test]
fn control_cmd_ping_rejects_pre_ble_active_scan_length() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 0,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 1,
        peer_slots_free: 2,
        peer_join_waiters: 1,
        remote_id_disable_mask: 0x1234_5678,
        ble_active_scan: 1,
    };
    let mut wire = pack_ping_vec(cmd).expect("pack ping");
    assert_eq!(wire.len(), 49);
    wire.truncate(48); // through remote_id only
    assert!(unpack_ping(&wire).is_err());
}
