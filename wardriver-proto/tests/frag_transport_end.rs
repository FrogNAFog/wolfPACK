//! PROTO v6 dump-frag `flags` / [`FRAG_FLAG_TRANSPORT_END`] (wolfPACK).

use wardriver_proto::{
    pack_ble_scan_frag, pack_probe_req_frag, pack_scan_frag, unpack_ble_scan_frag,
    unpack_probe_req_frag, unpack_scan_frag, ApEntryV1, AuthMini, BleEntryV1, BleScanFragV1Header,
    ProbeReqBurstV1, ProbeReqFragV1Header, ScanFragV1Header, FRAG_FLAG_TRANSPORT_END,
};

#[test]
fn scan_frag_transport_end_roundtrip() {
    let hdr = ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 9,
        seq: 1,
        scan_batch_id: 2,
        sniffer_boot_id: 3,
        sniffer_scan_end_ms: 4,
        sniffer_scan_end_unix_ms: 5,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 0,
    };
    let wire = pack_scan_frag(hdr, &[]).expect("pack scan frag");
    assert_eq!(wire[10], FRAG_FLAG_TRANSPORT_END);
    let (back, aps) = unpack_scan_frag(&wire).expect("unpack scan frag");
    assert_eq!(back.flags, FRAG_FLAG_TRANSPORT_END);
    assert_eq!(back.sniffer_id, hdr.sniffer_id);
    assert!(aps.is_empty());
}

#[test]
fn ble_scan_frag_transport_end_roundtrip() {
    let entry = BleEntryV1 {
        addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        rssi: -70,
        addr_type: 1,
        company_id: 0x004C,
        name: [0u8; 22],
        obs_mono_ms: 100,
    };
    let hdr = BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 2,
        seq: 3,
        scan_batch_id: 4,
        sniffer_boot_id: 5,
        sniffer_scan_end_ms: 6,
        sniffer_scan_end_unix_ms: 7,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 1,
    };
    let wire = pack_ble_scan_frag(hdr, &[entry]).expect("pack ble frag");
    assert_eq!(wire[10], FRAG_FLAG_TRANSPORT_END);
    let (back, entries) = unpack_ble_scan_frag(&wire).expect("unpack ble frag");
    assert_eq!(back.flags, FRAG_FLAG_TRANSPORT_END);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], entry);
}

#[test]
fn probe_req_frag_transport_end_roundtrip() {
    let entry = ProbeReqBurstV1 {
        src: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
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
    let wire = pack_probe_req_frag(hdr, &[entry]).expect("pack probe frag");
    assert_eq!(wire[10], FRAG_FLAG_TRANSPORT_END);
    let (back, entries) = unpack_probe_req_frag(&wire).expect("unpack probe frag");
    assert_eq!(back.flags, FRAG_FLAG_TRANSPORT_END);
    assert_eq!(entries[0], entry);
}

#[test]
fn scan_frag_with_ap_transport_end_roundtrip() {
    let ap = ApEntryV1 {
        bssid: [0x70, 0xC9, 0x4E, 0x00, 0x11, 0x22],
        channel: 11,
        rssi: -65,
        auth: AuthMini::Wpa2,
        ssid_len: 4,
        ssid: {
            let mut s = [0u8; 32];
            s[..4].copy_from_slice(b"test");
            s
        },
        obs_mono_ms: 1234,
    };
    let hdr = ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 99,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 1,
    };
    let wire = pack_scan_frag(hdr, &[ap]).expect("pack");
    let (back, aps) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(back.flags, FRAG_FLAG_TRANSPORT_END);
    assert_eq!(aps[0], ap);
}

// --- Test migrated from wire_edge.rs (renamed to avoid collision) ---

/// ScanFrag with FRAG_FLAG_TRANSPORT_END roundtrips (with AP data).
#[test]
fn scan_frag_transport_end_flag_on_last_chunk() {
    let header = ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 1,
        seq: 2,
        scan_batch_id: 5,
        sniffer_boot_id: 3,
        sniffer_scan_end_ms: 200,
        sniffer_scan_end_unix_ms: 1_700_000_000_000,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 1,
    };
    let mut ssid = [0u8; 32];
    ssid[..4].copy_from_slice(b"ABCD");
    let ap = ApEntryV1 {
        bssid: [2u8; 6],
        channel: 6,
        rssi: -45,
        auth: AuthMini::Wpa2,
        ssid_len: 4,
        ssid,
        obs_mono_ms: 100,
    };
    let frame = pack_scan_frag(header, &[ap]).expect("pack");
    let (h, aps) = unpack_scan_frag(&frame).expect("unpack");
    assert_eq!(h.flags, FRAG_FLAG_TRANSPORT_END);
    assert_eq!(aps.len(), 1);
}
