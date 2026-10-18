//! Golden round-trip tests for v1 wire layouts.

use wardriver_proto::{
    pack_control_cmd_vec, pack_control_welcome, pack_detection_alert, pack_detection_ble_alert,
    pack_ping_vec, pack_probe_req_frag, pack_sniffer_ack, time_from_cmd, unpack_control_cmd,
    unpack_control_welcome, unpack_detection_alert, unpack_detection_ble_alert, unpack_ping,
    unpack_probe_req_frag, unpack_sniffer_ack, ControlCmd, ControlCmdV1, ControlWelcomeV1,
    DetectionAlertV1, DetectionBleAlertV1, ProbeReqBurstV1, ProbeReqFragV1Header, SnifferAckV1,
    SnifferRole, SnifferState, CHANNELS_MAX, PROTO_VERSION,
};

#[test]
fn control_cmd_v1_roundtrip() {
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
        detection_disable_wifi_mask: 0x0000_0001,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0x0000_0004,
        desired_scanning: 0,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let wire = pack_control_cmd_vec(cmd).expect("pack control cmd");
    assert_eq!(wire[0], PROTO_VERSION);
    let back = unpack_control_cmd(&wire).expect("unpack control cmd");
    assert_eq!(back.cmd as u8, cmd.cmd as u8);
    assert_eq!(back.cmd_seq, cmd.cmd_seq);
    assert_eq!(back.control_channel, cmd.control_channel);
    assert_eq!(back.dwell_ms, cmd.dwell_ms);
    assert_eq!(back.chan_count, cmd.chan_count);
    assert_eq!(back.channels, cmd.channels);
    assert_eq!(back.desired_role, cmd.desired_role);
    assert_eq!(back.control_boot_id, cmd.control_boot_id);
    assert_eq!(
        back.detection_disable_wifi_mask,
        cmd.detection_disable_wifi_mask
    );
    assert_eq!(back.wardrive_suppressed, cmd.wardrive_suppressed);
    assert_eq!(back.time_set_seq, cmd.time_set_seq);
    assert_eq!(back.ctrl_unix_ms, cmd.ctrl_unix_ms);
    assert_eq!(
        back.detection_disable_ble_mask,
        cmd.detection_disable_ble_mask
    );
    assert_eq!(back.desired_scanning, cmd.desired_scanning);
    assert_eq!(back.remote_id_disable_mask, cmd.remote_id_disable_mask);
}

#[test]
fn control_cmd_v1_time_piggyback_roundtrip() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 7,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 1,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 99,
        ctrl_unix_ms: 1_700_000_000_000,
        detection_disable_ble_mask: 0,
        desired_scanning: 0,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let back = unpack_ping(&pack_ping_vec(cmd).expect("pack ping")).expect("unpack");
    assert_eq!(back.time_set_seq, 99);
    assert_eq!(back.ctrl_unix_ms, 1_700_000_000_000);
    let ts = time_from_cmd(&back).expect("time_from_cmd");
    assert_eq!(ts.seq, 99);
    assert_eq!(ts.ctrl_unix_ms, 1_700_000_000_000);
    assert_eq!(ts.ctrl_tx_mono_ms32, 0);
}

#[test]
fn control_cmd_v1_time_from_cmd_none_when_seq_zero() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 1,
        control_channel: 0,
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
    assert!(time_from_cmd(&cmd).is_none());
}

#[test]
fn control_welcome_v1_roundtrip() {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[..3].copy_from_slice(&[1, 6, 11]);
    let welcome = ControlWelcomeV1 {
        control_boot_id: 0x1122_3344,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        desired_scanning: 1,
        dwell_ms: 150,
        chan_count: 3,
        channels,
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        detection_disable_wifi_mask: 0x0000_0002,
        wardrive_suppressed: 0,
        fleet_slot: 3,
        detection_disable_ble_mask: 0x0000_0008,
        remote_id_disable_mask: 0x0000_0010,
        ble_active_scan: 0,
    };
    let wire = pack_control_welcome(welcome);
    assert_eq!(wire[0], PROTO_VERSION);
    let back = unpack_control_welcome(&wire).expect("unpack control welcome");
    assert_eq!(back.control_boot_id, welcome.control_boot_id);
    assert_eq!(back.control_channel, welcome.control_channel);
    assert_eq!(back.desired_scanning, welcome.desired_scanning);
    assert_eq!(back.dwell_ms, welcome.dwell_ms);
    assert_eq!(back.chan_count, welcome.chan_count);
    assert_eq!(back.channels, welcome.channels);
    assert_eq!(back.proto_min, welcome.proto_min);
    assert_eq!(back.proto_max, welcome.proto_max);
    assert_eq!(
        back.detection_disable_wifi_mask,
        welcome.detection_disable_wifi_mask
    );
    assert_eq!(back.wardrive_suppressed, welcome.wardrive_suppressed);
    assert_eq!(back.fleet_slot, welcome.fleet_slot);
    assert_eq!(
        back.detection_disable_ble_mask,
        welcome.detection_disable_ble_mask
    );
    assert_eq!(back.remote_id_disable_mask, welcome.remote_id_disable_mask);
    assert_eq!(back.ble_active_scan, welcome.ble_active_scan);
}

#[test]
fn control_welcome_v1_roundtrip_fleet_slot_48() {
    // Pins the P4-F1 fix: `fleet_slot` is an opaque u8 in 1..=48 and must NOT be
    // clamped to 7 on pack or unpack. Slot 48 is the fleet max (48 sniffers).
    let mut channels = [0u8; CHANNELS_MAX];
    channels[..3].copy_from_slice(&[1, 6, 11]);
    let welcome = ControlWelcomeV1 {
        control_boot_id: 0xAABB_CCDD,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        desired_scanning: 1,
        dwell_ms: 150,
        chan_count: 3,
        channels,
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        detection_disable_wifi_mask: 0x0000_0002,
        wardrive_suppressed: 0,
        fleet_slot: 48,
        detection_disable_ble_mask: 0x0000_0008,
        remote_id_disable_mask: 0x0000_0010,
        ble_active_scan: 0,
    };
    let wire = pack_control_welcome(welcome);
    // Wire layout: common header (10) + control_boot_id (4) + control_channel (1)
    //   + desired_scanning (1) + dwell_ms (2) + chan_count (1) + channels (16)
    //   + proto_min (1) + proto_max (1) + detection_disable_wifi_mask (4)
    //   + wardrive_suppressed (1)  => fleet_slot is byte 36.
    let fleet_slot_off = 10 + 4 + 1 + 1 + 2 + 1 + CHANNELS_MAX + 2 + 4 + 1;
    assert_eq!(
        wire[fleet_slot_off], 48,
        "packed fleet_slot must be 48, not clamped"
    );
    let back = unpack_control_welcome(&wire).expect("unpack control welcome slot 48");
    assert_eq!(
        back.fleet_slot, 48,
        "unpacked fleet_slot must be 48, not clamped to 7"
    );
}

#[test]
fn control_welcome_v1_rejects_truncated() {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[..3].copy_from_slice(&[1, 6, 11]);
    let welcome = ControlWelcomeV1 {
        control_boot_id: 0x1122_3344,
        control_channel: wardriver_proto::CONTROL_AP_CHANNEL,
        desired_scanning: 1,
        dwell_ms: 150,
        chan_count: 3,
        channels,
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        fleet_slot: 5,
        detection_disable_ble_mask: 0x10,
        remote_id_disable_mask: 0,
        ble_active_scan: 1,
    };
    let mut wire = pack_control_welcome(welcome);
    // Drop trailing ble_active_scan.
    wire.truncate(wire.len() - 1);
    assert!(unpack_control_welcome(&wire).is_err());
}

#[test]
fn detection_alert_v1_roundtrip() {
    let alert = DetectionAlertV1 {
        sniffer_id: 3,
        seq: 9,
        sniffer_boot_id: 0xDEAD_BEEF,
        sniffer_scan_end_ms: 50_000,
        sniffer_scan_end_unix_ms: 1_700_000_000_000,
        src: [0x70, 0xC9, 0x4E, 0x12, 0x34, 0x56],
        detection_method: 2,
        channel: 6,
        rssi_min: -82,
        rssi_max: -71,
        rssi_avg_q8: -76 << 8,
        wildcard_count: 4,
        first_seen_ms32: 49_900,
        last_seen_ms32: 50_000,
        primary_sig_matched: 1,
    };
    let wire = pack_detection_alert(alert);
    assert_eq!(wire[0], PROTO_VERSION);
    let back = unpack_detection_alert(&wire).expect("unpack flock alert");
    assert_eq!(back, alert);
}

#[test]
fn ble_detection_alert_v1_roundtrip() {
    let alert = DetectionBleAlertV1 {
        sniffer_id: 2,
        seq: 11,
        sniffer_boot_id: 0xCAFE_BABE,
        sniffer_scan_end_ms: 60_000,
        sniffer_scan_end_unix_ms: 1_700_000_100_000,
        src: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        detection_method: 32,
        company_id: 0x004C,
        rssi: -68,
    };
    let wire = pack_detection_ble_alert(alert);
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire[1], wardriver_proto::FrameType::DetectionBleAlert as u8);
    assert_eq!(
        wire.len(),
        10 + wardriver_proto::BLE_DETECTION_ALERT_BODY_SIZE
    );
    let back = unpack_detection_ble_alert(&wire).expect("unpack ble flock alert");
    assert_eq!(back, alert);
}

#[test]
fn probe_req_burst_v1_slim_roundtrip() {
    let entry = ProbeReqBurstV1 {
        src: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
        wildcard_count: 3,
        channel: 11,
        rssi_min: -90,
        rssi_max: -75,
        rssi_avg_q8: -80 << 8,
        first_seen_ms32: 1000,
        last_seen_ms32: 1200,
        primary_sig_matched: 1,
    };
    let hdr = ProbeReqFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 2,
        scan_batch_id: 3,
        sniffer_boot_id: 4,
        sniffer_scan_end_ms: 5000,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 1,
    };
    let wire = pack_probe_req_frag(hdr, &[entry]).expect("pack probe frag");
    let (back_hdr, back_entries) = unpack_probe_req_frag(&wire).expect("unpack probe frag");
    assert_eq!(back_hdr.entry_count, hdr.entry_count);
    assert_eq!(back_entries.len(), 1);
    assert_eq!(back_entries[0], entry);
}

#[test]
fn sniffer_ack_v1_bulk_lost_cycles_roundtrip() {
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
        bulk_lost_cycles: 3,
    };
    let wire = pack_sniffer_ack(ack);
    assert_eq!(wire[0], PROTO_VERSION);
    let back = unpack_sniffer_ack(&wire).expect("unpack sniffer ack");
    assert_eq!(back.sniffer_id, ack.sniffer_id);
    assert_eq!(back.ack_cmd_seq, ack.ack_cmd_seq);
    assert_eq!(back.state, ack.state);
    assert_eq!(back.role, ack.role);
    assert_eq!(back.ble_capable, ack.ble_capable);
    assert_eq!(back.sniffer_boot_id, ack.sniffer_boot_id);
    assert_eq!(back.free_heap_bytes, ack.free_heap_bytes);
    assert_eq!(back.total_heap_bytes, ack.total_heap_bytes);
    assert_eq!(back.cpu_load_pct, ack.cpu_load_pct);
    assert_eq!(back.five_ghz_capable, ack.five_ghz_capable);
    assert_eq!(back.dfs_capable, ack.dfs_capable);
    assert_eq!(back.bulk_lost_cycles, ack.bulk_lost_cycles);
}

#[test]
fn sniffer_ack_v1_bulk_lost_cycles_backward_compat() {
    let ack = SnifferAckV1 {
        sniffer_id: 1,
        ack_cmd_seq: 2,
        state: SnifferState::Idle,
        role: SnifferRole::Wifi,
        ble_capable: 0,
        sniffer_boot_id: 0,
        free_heap_bytes: 0,
        total_heap_bytes: 0,
        cpu_load_pct: 0,
        five_ghz_capable: 0,
        dfs_capable: 0,
        bulk_lost_cycles: 99,
    };
    let mut wire = pack_sniffer_ack(ack);
    wire.truncate(36); // pre-bulk_lost_cycles ack (COMMON_HDR + fixed fields)
    let back = unpack_sniffer_ack(&wire).expect("unpack legacy ack");
    assert_eq!(back.bulk_lost_cycles, 0);
}
