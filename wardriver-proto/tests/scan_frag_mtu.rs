//! ESP-NOW v2 `ScanFrag` size budget (wolfPACK variable SSID).

use wardriver_proto::{
    pack_scan_frag, unpack_scan_frag, ApEntryV1, AuthMini, ScanFragV1Header, ESP_NOW_MAX_FRAME_LEN,
    MAX_APS_PER_FRAG_WORST_CASE,
};

fn max_ssid_ap(bssid_byte: u8) -> ApEntryV1 {
    ApEntryV1 {
        bssid: [bssid_byte; 6],
        channel: 6,
        rssi: -70,
        auth: AuthMini::Wpa2,
        ssid_len: 32,
        ssid: [0x41u8; 32],
        obs_mono_ms: 1,
    }
}

#[test]
fn worst_case_single_frag_within_espnow_v2_mtu() {
    let aps: Vec<ApEntryV1> = (0..MAX_APS_PER_FRAG_WORST_CASE)
        .map(|i| max_ssid_ap(i as u8))
        .collect();
    let hdr = ScanFragV1Header {
        flags: 0,
        sniffer_id: 1,
        seq: 1,
        scan_batch_id: 1,
        sniffer_boot_id: 1,
        sniffer_scan_end_ms: 0,
        sniffer_scan_end_unix_ms: 0,
        frag_idx: 0,
        frag_total: 1,
        ap_count: aps.len() as u8,
    };
    let wire = pack_scan_frag(hdr, &aps).expect("pack worst-case frag");
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
    let (back, back_aps) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(back.ap_count as usize, aps.len());
    assert_eq!(back_aps, aps);
}
