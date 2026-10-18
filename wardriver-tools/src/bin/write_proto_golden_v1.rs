//! Host utility: write `wardriver-proto/tests/golden/v1/*.bin` fixtures.
//!
//! Run from repo root:
//! `cargo run -p wardriver-tools --bin write_proto_golden_v1`
//!
//! For 5 GHz control channel golden:
//! `cargo run -p wardriver-tools --bin write_proto_golden_v1 --features mgmt-ap-5ghz`

use std::path::PathBuf;

use wardriver_proto::{
    pack_ble_scan_frag, pack_control_cmd_vec, pack_ping_vec, pack_probe_req_frag, pack_scan_frag,
    pack_sniffer_ack, ApEntryV1, AuthMini, BleEntryV1, BleScanFragV1Header, ControlCmd,
    ControlCmdV1, ProbeReqBurstV1, ProbeReqFragV1Header, ScanFragV1Header, SnifferAckV1,
    SnifferRole, SnifferState, CHANNELS_MAX, FRAG_FLAG_TRANSPORT_END,
};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = root.join("../wardriver-proto/tests/golden/v1");
    std::fs::create_dir_all(&out_dir).expect("mkdir golden/v1");

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
    let cmd_wire = pack_control_cmd_vec(cmd).expect("pack control cmd");
    #[cfg(not(feature = "mgmt-ap-5ghz"))]
    std::fs::write(out_dir.join("control_cmd_v1.bin"), &cmd_wire)
        .expect("write control_cmd_v1.bin");
    #[cfg(feature = "mgmt-ap-5ghz")]
    std::fs::write(out_dir.join("control_cmd_v1_5ghz.bin"), &cmd_wire)
        .expect("write control_cmd_v1_5ghz.bin");

    let ping = ControlCmdV1 {
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
    let ping_wire = pack_ping_vec(ping).expect("pack ping");
    std::fs::write(out_dir.join("control_cmd_ping_v7.bin"), &ping_wire)
        .expect("write control_cmd_ping_v7.bin");

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
    std::fs::write(out_dir.join("sniffer_ack_v1.bin"), pack_sniffer_ack(ack))
        .expect("write sniffer_ack_v1.bin");

    // Fixed 5 GHz AP in a single ScanFrag (pipeline / wire regression).
    let mut ssid = [0u8; 32];
    ssid[..10].copy_from_slice(b"FiveG-Test");
    let ap = ApEntryV1 {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x36],
        channel: 36,
        rssi: -62,
        auth: AuthMini::Wpa2,
        ssid_len: 10,
        ssid,
        obs_mono_ms: 49_500,
    };
    let scan_hdr = ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 3,
        seq: 10,
        scan_batch_id: 20,
        sniffer_boot_id: 0xAABB_CCDD,
        sniffer_scan_end_ms: 50_000,
        sniffer_scan_end_unix_ms: 1_777_190_400_000,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 1,
    };
    std::fs::write(
        out_dir.join("scan_frag_v1_5ghz_ap.bin"),
        pack_scan_frag(scan_hdr, std::slice::from_ref(&ap)).expect("pack scan frag"),
    )
    .expect("write scan_frag_v1_5ghz_ap.bin");

    let ble_entry = BleEntryV1 {
        addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        rssi: -70,
        addr_type: 1,
        company_id: 0x004C,
        name: {
            let mut n = [0u8; 22];
            n[..10].copy_from_slice(b"Pixel Buds");
            n
        },
        obs_mono_ms: 30_000,
    };
    let ble_hdr = BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 5,
        seq: 7,
        scan_batch_id: 9,
        sniffer_boot_id: 0x1122_3344,
        sniffer_scan_end_ms: 30_000,
        sniffer_scan_end_unix_ms: 1_777_188_600_000,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 1,
    };
    std::fs::write(
        out_dir.join("ble_scan_frag_v1.bin"),
        pack_ble_scan_frag(ble_hdr, std::slice::from_ref(&ble_entry)).expect("pack ble frag"),
    )
    .expect("write ble_scan_frag_v1.bin");

    let probe_entry = ProbeReqBurstV1 {
        src: [0x70, 0xC9, 0x4E, 0x12, 0x34, 0x56],
        wildcard_count: 2,
        channel: 6,
        rssi_min: -88,
        rssi_max: -72,
        rssi_avg_q8: -80 << 8,
        first_seen_ms32: 50,
        last_seen_ms32: 60,
        primary_sig_matched: 0,
    };
    let probe_hdr = ProbeReqFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
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
    std::fs::write(
        out_dir.join("probe_req_frag_v1.bin"),
        pack_probe_req_frag(probe_hdr, std::slice::from_ref(&probe_entry))
            .expect("pack probe frag"),
    )
    .expect("write probe_req_frag_v1.bin");

    println!("wrote {}", out_dir.display());
}
