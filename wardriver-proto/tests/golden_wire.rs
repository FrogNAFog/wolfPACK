//! Stable v1 wire bytes for ESP-NOW frag types (regression goldens).

use wardriver_proto::{
    pack_ble_scan_frag, pack_probe_req_frag, pack_scan_frag, unpack_ble_scan_frag,
    unpack_probe_req_frag, unpack_scan_frag, ApEntryV1, AuthMini, BleEntryV1, BleScanFragV1Header,
    ProbeReqBurstV1, ProbeReqFragV1Header, ScanFragV1Header, FRAG_FLAG_TRANSPORT_END,
};

fn sample_5ghz_ap() -> ApEntryV1 {
    let mut ssid = [0u8; 32];
    ssid[..10].copy_from_slice(b"FiveG-Test");
    ApEntryV1 {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x36],
        channel: 36,
        rssi: -62,
        auth: AuthMini::Wpa2,
        ssid_len: 10,
        ssid,
        obs_mono_ms: 49_500,
    }
}

fn sample_scan_hdr() -> ScanFragV1Header {
    ScanFragV1Header {
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
    }
}

#[test]
fn golden_scan_frag_v1_5ghz_ap_bytes() {
    let wire = pack_scan_frag(sample_scan_hdr(), std::slice::from_ref(&sample_5ghz_ap()))
        .expect("pack scan frag");
    let golden = include_bytes!("golden/v1/scan_frag_v1_5ghz_ap.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "scan_frag_v1_5ghz_ap wire changed; refresh via write_proto_golden_v1"
    );
    let (_hdr, aps) = unpack_scan_frag(golden).expect("unpack golden");
    assert_eq!(aps[0].channel, 36);
}

#[test]
fn golden_ble_scan_frag_v1_bytes() {
    let mut name = [0u8; 22];
    name[..10].copy_from_slice(b"Pixel Buds");
    let entry = BleEntryV1 {
        addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        rssi: -70,
        addr_type: 1,
        company_id: 0x004C,
        name,
        obs_mono_ms: 30_000,
    };
    let hdr = BleScanFragV1Header {
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
    let wire = pack_ble_scan_frag(hdr, std::slice::from_ref(&entry)).expect("pack ble");
    let golden = include_bytes!("golden/v1/ble_scan_frag_v1.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "ble_scan_frag_v1 wire changed; refresh via write_proto_golden_v1"
    );
    let (_, entries) = unpack_ble_scan_frag(golden).expect("unpack golden");
    assert_eq!(entries.len(), 1);
}

#[test]
fn golden_probe_req_frag_v1_bytes() {
    let entry = ProbeReqBurstV1 {
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
    let hdr = ProbeReqFragV1Header {
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
    let wire = pack_probe_req_frag(hdr, std::slice::from_ref(&entry)).expect("pack probe");
    let golden = include_bytes!("golden/v1/probe_req_frag_v1.bin");
    assert_eq!(
        wire.as_slice(),
        golden,
        "probe_req_frag_v1 wire changed; refresh via write_proto_golden_v1"
    );
    let (_, entries) = unpack_probe_req_frag(golden).expect("unpack golden");
    assert_eq!(entries[0].channel, 6);
}
