//! SSID denylist wire round-trip and size limits.

use wardriver_proto::{
    pack_ssid_denylist, unpack_ssid_denylist, ESP_NOW_MAX_FRAME_LEN, SSID_DENYLIST_HDR_SIZE,
    SSID_DENYLIST_MAX_SSID_BYTES,
};

#[test]
fn ssid_denylist_roundtrip() {
    let ssids = vec![
        "home-net".to_string(),
        "Guest_WiFi".to_string(),
        String::from_utf8(vec![0xC3, 0xA9, 0x74, 0x61, 0x67, 0x65]).unwrap(), // "étage"
    ];
    let wire = pack_ssid_denylist(&ssids).expect("pack");
    let back = unpack_ssid_denylist(&wire).expect("unpack");
    assert_eq!(back, ssids);
}

#[test]
fn ssid_denylist_empty() {
    let wire = pack_ssid_denylist(&[]).expect("pack empty");
    assert_eq!(wire.len(), SSID_DENYLIST_HDR_SIZE);
    let back = unpack_ssid_denylist(&wire).expect("unpack empty");
    assert!(back.is_empty());
}

#[test]
fn ssid_denylist_large_pack_ok() {
    // 8 × 32-byte SSIDs fit in one ESP-NOW v2 frame (were >250 B on v1).
    let big = "x".repeat(SSID_DENYLIST_MAX_SSID_BYTES);
    let ssids: Vec<String> = (0..8).map(|_| big.clone()).collect();
    let wire = pack_ssid_denylist(&ssids).expect("pack large denylist");
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
    let back = unpack_ssid_denylist(&wire).expect("unpack");
    assert_eq!(back, ssids);
}

#[test]
fn ssid_denylist_max_entries_pack_ok() {
    // Full 32-entry denylist stays under v2 MTU.
    let big = "y".repeat(SSID_DENYLIST_MAX_SSID_BYTES);
    let ssids: Vec<String> = (0..32).map(|_| big.clone()).collect();
    let wire = pack_ssid_denylist(&ssids).expect("pack max denylist");
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
}
