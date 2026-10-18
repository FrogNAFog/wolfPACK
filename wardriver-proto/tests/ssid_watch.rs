//! SSID watchlist + alert wire roundtrips.

use wardriver_proto::{
    pack_ssid_watch_alert, pack_ssid_watchlist, unpack_ssid_watch_alert, unpack_ssid_watchlist,
    SsidWatchAlertV1, ESP_NOW_MAX_FRAME_LEN, SSID_DENYLIST_HDR_SIZE, SSID_DENYLIST_MAX_SSID_BYTES,
};

#[test]
fn ssid_watchlist_roundtrip() {
    let ssids = vec!["HomeNet".to_string(), "Guest_WiFi".to_string()];
    let wire = pack_ssid_watchlist(&ssids).expect("pack");
    let back = unpack_ssid_watchlist(&wire).expect("unpack");
    assert_eq!(back, ssids);
}

#[test]
fn ssid_watchlist_empty() {
    let wire = pack_ssid_watchlist(&[]).expect("pack empty");
    assert_eq!(wire.len(), SSID_DENYLIST_HDR_SIZE);
    let back = unpack_ssid_watchlist(&wire).expect("unpack empty");
    assert!(back.is_empty());
}

#[test]
fn ssid_watch_alert_roundtrip() {
    let alert = SsidWatchAlertV1 {
        sniffer_id: 42,
        sniffer_boot_id: 7,
        channel: 11,
        rssi: -65,
        bssid: [0xde, 0xad, 0xbe, 0xef, 0x00, 0x01],
        ssid: "WatchMe".to_string(),
    };
    let wire = pack_ssid_watch_alert(&alert).expect("pack");
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
    let back = unpack_ssid_watch_alert(&wire).expect("unpack");
    assert_eq!(back, alert);
}

#[test]
fn ssid_watch_alert_max_ssid_bytes() {
    let big = "x".repeat(SSID_DENYLIST_MAX_SSID_BYTES);
    let alert = SsidWatchAlertV1 {
        sniffer_id: 1,
        sniffer_boot_id: 1,
        channel: 6,
        rssi: -70,
        bssid: [0; 6],
        ssid: big.clone(),
    };
    let wire = pack_ssid_watch_alert(&alert).expect("pack max ssid");
    let back = unpack_ssid_watch_alert(&wire).expect("unpack");
    assert_eq!(back.ssid, big);
}
