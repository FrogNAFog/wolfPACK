//! TLV roundtrip on scan/BLE fragments; legacy goldens still unpack.

use wardriver_proto::{
    chunk_aps_for_scan_frag_ranges, count_scan_frag_chunks_with_extras,
    pack_ble_scan_frag_with_extras, pack_scan_frag_with_extras, unpack_ble_scan_frag,
    unpack_ble_scan_frag_with_extras, unpack_scan_frag, unpack_scan_frag_with_extras, ApEntryV1,
    AuthMini, BleEntryV1, BleScanFragV1Header, ScanEntryExtras, ScanFragV1Header,
    FRAG_FLAG_TRANSPORT_END, SCAN_TLV_TAG_BLE_COD, SCAN_TLV_TAG_RCOI_IE,
};

fn sample_ap() -> ApEntryV1 {
    let mut ssid = [0u8; 32];
    ssid[..5].copy_from_slice(b"Snoop");
    ApEntryV1 {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        channel: 6,
        rssi: -45,
        auth: AuthMini::Wpa2,
        ssid_len: 5,
        ssid,
        obs_mono_ms: 30_000,
    }
}

#[test]
fn legacy_golden_scan_frag_still_unpacks() {
    let golden = include_bytes!("golden/v1/scan_frag_v1_5ghz_ap.bin");
    let (_hdr, aps) = unpack_scan_frag(golden).expect("legacy golden");
    assert_eq!(aps.len(), 1);
}

#[test]
fn scan_frag_tlv_roundtrip_rcoi() {
    let ap = sample_ap();
    let extras = ScanEntryExtras {
        rcoi_ie_payload: vec![0x03, 0x55, 0x5A, 0x03, 0xBA],
        ble_cod: None,
    };
    let hdr = ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 1,
        seq: 2,
        scan_batch_id: 3,
        sniffer_boot_id: 4,
        sniffer_scan_end_ms: 30_000,
        sniffer_scan_end_unix_ms: 1_777_188_600_000,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 1,
    };
    let wire = pack_scan_frag_with_extras(
        hdr,
        std::slice::from_ref(&ap),
        std::slice::from_ref(&extras),
    )
    .expect("pack tlv");
    let (back_hdr, back_aps, back_extras) =
        unpack_scan_frag_with_extras(&wire).expect("unpack tlv");
    assert_eq!(back_hdr.ap_count, 1);
    assert_eq!(back_aps[0].bssid, ap.bssid);
    assert_eq!(back_extras[0].rcoi_ie_payload, extras.rcoi_ie_payload);
    // Legacy unpack ignores TLV but still reads AP rows.
    let (_legacy_hdr, legacy_aps) = unpack_scan_frag(&wire).expect("legacy unpack with tlv");
    assert_eq!(legacy_aps[0].bssid, ap.bssid);
}

#[test]
fn ble_frag_tlv_roundtrip_cod() {
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
    let extras = ScanEntryExtras {
        rcoi_ie_payload: Vec::new(),
        ble_cod: Some([0x18, 0x04, 0x00]),
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
    let wire = pack_ble_scan_frag_with_extras(
        hdr,
        std::slice::from_ref(&entry),
        std::slice::from_ref(&extras),
    )
    .expect("pack ble tlv");
    let (_back_hdr, back_entries, back_extras) =
        unpack_ble_scan_frag_with_extras(&wire).expect("unpack ble tlv");
    assert_eq!(back_entries[0].addr, entry.addr);
    assert_eq!(back_extras[0].ble_cod, extras.ble_cod);
    let (_legacy_hdr, legacy_entries) = unpack_ble_scan_frag(&wire).expect("legacy ble unpack");
    assert_eq!(legacy_entries[0].addr, entry.addr);
}

#[test]
fn tlv_tags_are_stable() {
    assert_eq!(SCAN_TLV_TAG_RCOI_IE, 0x01);
    assert_eq!(SCAN_TLV_TAG_BLE_COD, 0x03);
}

#[test]
fn extras_increase_scan_frag_chunk_count() {
    let aps: Vec<_> = (0..20)
        .map(|i| {
            let mut ap = sample_ap();
            ap.bssid[5] = i;
            ap
        })
        .collect();
    let fat = ScanEntryExtras {
        rcoi_ie_payload: vec![0xAA; 200],
        ble_cod: None,
    };
    let extras = vec![fat; aps.len()];
    let without = count_scan_frag_chunks_with_extras(&aps, &[]);
    let with = count_scan_frag_chunks_with_extras(&aps, &extras);
    assert!(with >= without);
    assert!(with > without);
    let ranges = chunk_aps_for_scan_frag_ranges(&aps, &extras);
    assert_eq!(ranges.len(), with);
}
