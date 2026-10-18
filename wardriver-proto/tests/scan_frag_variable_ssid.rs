//! Variable-length SSID on wolfPACK `ScanFrag` wire.

use wardriver_proto::{
    ap_entry_wire_len, chunk_aps_for_scan_frag_ranges, pack_ble_scan_frag, pack_scan_frag,
    unpack_ble_scan_frag, unpack_scan_frag, ApEntryV1, AuthMini, BleEntryV1, BleScanFragV1Header,
    ScanFragV1Header, SCAN_FRAG_ENTRY_BUDGET,
};

fn sample_hdr(ap_count: u8) -> ScanFragV1Header {
    ScanFragV1Header {
        flags: 0,
        sniffer_id: 42,
        seq: 7,
        scan_batch_id: 8,
        sniffer_boot_id: 9,
        sniffer_scan_end_ms: 100,
        sniffer_scan_end_unix_ms: 200,
        frag_idx: 0,
        frag_total: 1,
        ap_count,
    }
}

fn ap_with_ssid(bssid: u8, ssid: &[u8]) -> ApEntryV1 {
    let mut buf = [0u8; 32];
    let sl = ssid.len().min(32);
    buf[..sl].copy_from_slice(&ssid[..sl]);
    ApEntryV1 {
        bssid: [bssid; 6],
        channel: 11,
        rssi: -55,
        auth: AuthMini::Open,
        ssid_len: sl as u8,
        ssid: buf,
        obs_mono_ms: 999,
    }
}

#[test]
fn hidden_ssid_roundtrip() {
    let ap = ap_with_ssid(1, b"");
    assert_eq!(ap_entry_wire_len(&ap), 14);
    let wire = pack_scan_frag(sample_hdr(1), std::slice::from_ref(&ap)).expect("pack");
    let (_, aps) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(aps[0], ap);
}

#[test]
fn short_ssid_roundtrip() {
    let ap = ap_with_ssid(2, b"cafe-guest");
    let wire = pack_scan_frag(sample_hdr(1), std::slice::from_ref(&ap)).expect("pack");
    let (_, aps) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(aps[0], ap);
}

#[test]
fn max_ssid_roundtrip() {
    let ap = ap_with_ssid(3, &[0xABu8; 32]);
    assert_eq!(ap_entry_wire_len(&ap), 14 + 32);
    let wire = pack_scan_frag(sample_hdr(1), std::slice::from_ref(&ap)).expect("pack");
    let (_, aps) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(aps[0], ap);
}

#[test]
fn mixed_ap_single_frag() {
    // Hidden + short + medium SSIDs fit one frame; fixed 32-byte stride would need more.
    let aps = vec![
        ap_with_ssid(10, b""),
        ap_with_ssid(11, b"hi"),
        ap_with_ssid(12, b"medium-ssid-name"),
    ];
    let ranges = chunk_aps_for_scan_frag_ranges(&aps, &[]);
    assert_eq!(ranges.len(), 1);
    let wire = pack_scan_frag(sample_hdr(aps.len() as u8), &aps).expect("pack");
    let (_, back) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(back, aps);
}

#[test]
fn multi_frag_greedy_chunking() {
    // 31 max-length SSIDs exceed one frame body budget (30 × 46 B fits; 31st forces split).
    let mut aps: Vec<ApEntryV1> = (0..31).map(|i| ap_with_ssid(i, &[0x58u8; 32])).collect();
    let ranges = chunk_aps_for_scan_frag_ranges(&aps, &[]);
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].1 - ranges[0].0, 30);
    assert_eq!(ranges[1].1 - ranges[1].0, 1);

    // 32 max SSIDs need at least two frames.
    aps.push(ap_with_ssid(100, &[0x59u8; 32]));
    let ranges2 = chunk_aps_for_scan_frag_ranges(&aps, &[]);
    assert!(ranges2.len() >= 2);
}

#[test]
fn pack_rejects_over_budget() {
    // One AP cannot exceed entry budget with ssid_len <= 32; use too many max SSIDs in one pack.
    let aps: Vec<ApEntryV1> = (0..31).map(|i| ap_with_ssid(i, &[0x41u8; 32])).collect();
    let err = pack_scan_frag(sample_hdr(aps.len() as u8), &aps).unwrap_err();
    assert!(err.to_string().contains("over budget"));
    let _ = SCAN_FRAG_ENTRY_BUDGET;
}

#[test]
fn reject_trailing_bytes() {
    let ap = ap_with_ssid(1, b"x");
    let mut wire = pack_scan_frag(sample_hdr(1), std::slice::from_ref(&ap)).expect("pack");
    wire.push(0);
    assert!(unpack_scan_frag(&wire).is_err());
}

// --- Tests migrated from wire_edge.rs (renamed to avoid collision) ---

/// Unpack scan frag with extra trailing bytes should return error.
#[test]
fn scan_frag_reject_trailing_bytes() {
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
    let ap = ApEntryV1 {
        bssid: [1u8; 6],
        channel: 6,
        rssi: -50,
        auth: AuthMini::Open,
        ssid_len: 0,
        ssid: [0u8; 32],
        obs_mono_ms: 100,
    };
    let frame = pack_scan_frag(header, &[ap]).unwrap();
    let mut bad = frame;
    bad.extend_from_slice(&[0xFF, 0xFF]);
    assert!(unpack_scan_frag(&bad).is_err());
}

/// Unpack ble scan frag with extra trailing bytes should return error.
#[test]
fn unpack_ble_scan_frag_trailing_bytes() {
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
        entry_count: 1,
    };
    let frame = pack_ble_scan_frag(header, &[ble([0xBB; 6], -55)]).unwrap();
    let mut bad = frame;
    bad.extend_from_slice(&[0xFF, 0xFF]);
    assert!(unpack_ble_scan_frag(&bad).is_err());
}
