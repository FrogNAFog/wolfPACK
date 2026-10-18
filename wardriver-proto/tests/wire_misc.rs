//! Miscellaneous wire edge cases without a dedicated home file.

use wardriver_proto::{
    pack_ble_scan_frag, pack_control_cmd_vec, pack_control_welcome, pack_ping_vec,
    pack_post_dump_done, pack_probe_req_frag, pack_scan_frag, peer_proto_accepts_us,
    unpack_ble_scan_frag, unpack_control_cmd, unpack_control_welcome, unpack_ping,
    unpack_post_dump_done, unpack_probe_req_frag, unpack_scan_frag, ApEntryV1, AuthMini,
    BleEntryV1, BleScanFragV1Header, ControlCmd, ControlCmdV1, ControlWelcomeV1, PostDumpDoneV1,
    ProbeReqFragV1Header, ScanFragV1Header, CHANNELS_MAX, ESP_NOW_MAX_FRAME_LEN, PROTO_VERSION,
};

fn ap(
    bssid: [u8; 6],
    channel: u8,
    rssi: i8,
    auth: AuthMini,
    ssid_len: u8,
    ssid: [u8; 32],
) -> ApEntryV1 {
    ApEntryV1 {
        bssid,
        channel,
        rssi,
        auth,
        ssid_len,
        ssid,
        obs_mono_ms: 100,
    }
}

fn ble(addr: [u8; 6], rssi: i8) -> BleEntryV1 {
    BleEntryV1 {
        addr,
        rssi,
        addr_type: 0,
        company_id: 0,
        name: [0u8; 22],
        obs_mono_ms: 100,
    }
}

/// pack_scan_frag with zero APs should succeed.
#[test]
fn pack_scan_frag_zero_aps() {
    let header = ScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 0,
    };
    let result = pack_scan_frag(header, &[]);
    assert!(result.is_ok(), "zero APs should be valid");
    let frame = result.unwrap();
    let (h, aps) = unpack_scan_frag(&frame).expect("roundtrip");
    assert_eq!(h.ap_count, 0);
    assert!(aps.is_empty());
}

/// pack_ble_scan_frag with zero entries should succeed.
#[test]
fn pack_ble_scan_frag_zero_entries() {
    let header = BleScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 0,
    };
    let result = pack_ble_scan_frag(header, &[]);
    assert!(result.is_ok(), "zero BLE entries should be valid");
    let frame = result.unwrap();
    let (h, entries) = unpack_ble_scan_frag(&frame).expect("roundtrip");
    assert_eq!(h.entry_count, 0);
    assert!(entries.is_empty());
}

/// pack_ble_scan_frag with entry_count != entries.len() returns error.
#[test]
fn pack_ble_scan_frag_count_mismatch() {
    let header = BleScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 5,
    };
    let entries = vec![ble([0xAA; 6], -60)];
    let result = pack_ble_scan_frag(header, &entries);
    assert!(result.is_err());
}

/// pack_ble_scan_frag with 256 entries (exceeds u8 entry_count max) — returns error.
#[test]
fn pack_ble_scan_frag_256_entries_rejected() {
    let entries: Vec<BleEntryV1> = (0..256)
        .map(|i| {
            let mut addr = [0u8; 6];
            addr[5] = (i % 256) as u8;
            BleEntryV1 {
                addr,
                rssi: -60,
                addr_type: 0,
                company_id: 0,
                name: [0u8; 22],
                obs_mono_ms: 0,
            }
        })
        .collect();
    let header = BleScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 255, // max u8
    };
    // 256 entries with entry_count=255 → mismatch error
    let result = pack_ble_scan_frag(header, &entries);
    assert!(result.is_err(), "256 entries should fail (exceeds u8 max)");
}

/// pack_ble_scan_frag with entry_count > entries.len() — returns error.
#[test]
fn pack_ble_scan_frag_entry_count_exceeds_entries() {
    let header = BleScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 5,
    };
    let result = pack_ble_scan_frag(header, &[ble([1u8; 6], -60)]);
    assert!(
        result.is_err(),
        "entry_count=5 but only 1 entry should fail"
    );
}

/// pack_scan_frag with ap_count != aps.len() returns error.
#[test]
fn pack_scan_frag_ap_count_mismatch() {
    let header = ScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 3,
    };
    let result = pack_scan_frag(
        header,
        &[ap([0xCC; 6], 1, -70, AuthMini::Open, 0, [0u8; 32])],
    );
    assert!(result.is_err());
}

/// ControlCmdV1 with chan_count = 0 is valid.
#[test]
fn control_cmd_zero_chan_count() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::StartScanning,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 200,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 42,
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
    let frame = pack_control_cmd_vec(cmd).expect("pack");
    let unpacked = unpack_control_cmd(&frame).expect("unpack");
    assert_eq!(unpacked.chan_count, 0);
}

/// ControlCmdV1 with chan_count > CHANNELS_MAX is clamped on wire.
#[test]
fn control_cmd_chan_count_overflow_clamped() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::StartScanning,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 200,
        chan_count: 255,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 42,
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
    let frame = pack_control_cmd_vec(cmd).expect("pack");
    let unpacked = unpack_control_cmd(&frame).expect("unpack");
    assert_eq!(unpacked.chan_count, CHANNELS_MAX as u8);
}

/// ControlWelcomeV1 with proto_min > proto_max — pack does not reject (no validation).
/// Unpack rejects inverted ranges as "proto version mismatch" because no peer version
/// can fall within an inverted [proto_min, proto_max] interval.
#[test]
fn control_welcome_inverted_proto_range_rejected() {
    let welcome = ControlWelcomeV1 {
        control_boot_id: 42,
        control_channel: 6,
        desired_scanning: 1,
        dwell_ms: 300,
        chan_count: 3,
        channels: [
            1, 6, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ],
        proto_min: 10,
        proto_max: 5, // Inverted: min > max
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        fleet_slot: 1,
        detection_disable_ble_mask: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let frame = pack_control_welcome(welcome);
    let result = unpack_control_welcome(&frame);
    assert!(result.is_err(), "unpack should reject inverted proto range");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("proto version mismatch"),
        "error should be proto version mismatch"
    );
    // Pack doesn't validate, but peer_proto_accepts_us rejects inverted ranges:
    assert!(
        !peer_proto_accepts_us(10, 5),
        "inverted range should reject any peer version"
    );
    // PROTO_VERSION (6) is NOT in [10, 5], so unpack correctly rejects.
    assert!(
        !peer_proto_accepts_us(PROTO_VERSION, PROTO_VERSION)
            || PROTO_VERSION >= 5 && PROTO_VERSION <= 10,
        "sanity: PROTO_VERSION={PROTO_VERSION}"
    );
}

/// peer_proto_accepts_us with proto_min > proto_max returns false.
#[test]
fn peer_proto_accepts_inverted_range() {
    assert!(!peer_proto_accepts_us(10, 5));
}

/// Wire version byte = 255 rejected by verify_common_header.
#[test]
fn wire_version_255_rejected() {
    let len = wardriver_proto::PEER_JOIN_FRAME_LEN;
    let mut frame = vec![0u8; len];
    frame[0] = 255;
    frame[1] = 19; // PeerJoin
    frame[2..6].copy_from_slice(b"WDRV");
    frame[6..10].copy_from_slice(&0x3257_4452u32.to_le_bytes());
    frame[10..14].copy_from_slice(&1u32.to_le_bytes());
    frame[14..18].copy_from_slice(&1u32.to_le_bytes());
    frame[18] = 0;
    assert!(wardriver_proto::unpack_peer_join(&frame).is_err());
}

/// Ping with peer_slots_free capped at 16 on wire.
#[test]
fn ping_peer_slots_free_capped() {
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq: 5,
        control_channel: 11,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id: 99,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 1,
        peer_slots_free: 200,
        peer_join_waiters: 3,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let frame = pack_ping_vec(cmd).expect("pack ping");
    let unpacked = unpack_ping(&frame).expect("unpack ping");
    assert_eq!(unpacked.peer_slots_free, 16);
    assert_eq!(unpacked.peer_join_waiters, 3);
}

/// ScanFrag frame within ESP_NOW_MAX_FRAME_LEN budget.
#[test]
fn scan_frag_within_budget() {
    let header = ScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 1,
    };
    let frame = pack_scan_frag(
        header,
        &[ap([0xDD; 6], 11, -65, AuthMini::Wpa2, 0, [0u8; 32])],
    )
    .unwrap();
    assert!(
        frame.len() <= ESP_NOW_MAX_FRAME_LEN,
        "frame exceeds ESP-NOW budget"
    );
}

/// ProbeReqFrag with zero entries should succeed.
#[test]
fn probe_req_frag_zero_entries() {
    let header = ProbeReqFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 0,
    };
    let result = pack_probe_req_frag(header, &[]);
    assert!(result.is_ok(), "zero probe entries should be valid");
    let frame = result.unwrap();
    let (h, entries) = unpack_probe_req_frag(&frame).expect("roundtrip");
    assert_eq!(h.entry_count, 0);
    assert!(entries.is_empty());
}

/// PostDumpDone unpack with short frame (was in wire_edge).
#[test]
fn unpack_post_dump_done_short_frame() {
    let short = vec![0u8; wardriver_proto::POST_DUMP_DONE_FRAME_LEN - 1];
    assert!(unpack_post_dump_done(&short).is_err());
}
