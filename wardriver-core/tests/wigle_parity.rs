//! Required WiGLE Android parity host tests (RCOI, CoD, CSV export shape).

use wardriver_core::ieee80211_parse::roaming_consortium_ie_payload_from_mgmt_mpdu;
use wardriver_core::wigle::{
    format_ble_csv_line, format_wifi_csv_line, WigleBleLineInput, WigleWifiLineInput,
};
use wardriver_core::wigle_ble::ble_csv_fields_from_cod;
use wardriver_core::wigle_rcoi::{format_rcoi, rcois_from_ie_payload};

#[test]
fn format_rcoi_boundaries_match_android() {
    assert_eq!(format_rcoi(0x5A03BA), "5A03BA");
    assert_eq!(format_rcoi(16_777_215), "FFFFFF");
    assert_eq!(format_rcoi(16_777_216), "0001000000");
    assert_eq!(format_rcoi(0x5A03BA0000), "5A03BA0000");
}

#[test]
fn openroaming_ie_payload_matches_android_comment() {
    let payload = [
        0x03, 0x55, 0x5A, 0x03, 0xBA, 0x00, 0x00, 0xBA, 0xA2, 0xD0, 0x00, 0x00, 0xBA, 0xA2, 0xD0,
        0x20, 0x00,
    ];
    assert_eq!(
        rcois_from_ie_payload(&payload),
        "5A03BA0000 BAA2D00000 BAA2D02000"
    );
}

#[test]
fn golden_wifi_row_doc_rcoi() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0x1a, 0x9f, 0xee, 0x5c, 0x71, 0xc6],
        ssid: "Scampoodle".to_string(),
        auth_mode: "[WPA2-EAP-CCMP][ESS]".to_string(),
        first_seen_utc: "2018-08-01 13:08:27".to_string(),
        channel: 161,
        frequency_mhz: 5805,
        rssi: -43,
        lat: Some(37.76578028),
        lon: Some(-123.45919439),
        altitude_m: Some(67),
        accuracy_m: Some(3.216),
        rcois: "5A03BA0000 BAA2D00000 BAA2D02000".to_string(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/wifi_row_doc_rcoi.line").trim()
    );
}

#[test]
fn golden_ble_row_headphones_cod() {
    let (capabilities, frequency_code) = ble_csv_fields_from_cod(0x18, 0x04, 0x00);
    let line = format_ble_csv_line(&WigleBleLineInput {
        addr: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        name: "BT Headset".to_string(),
        capabilities,
        first_seen_utc: "2026-04-26 08:15:00".to_string(),
        channel: None,
        frequency_code: Some(frequency_code),
        rssi: -55,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "BLE",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/ble_row_headphones.line").trim()
    );
}

#[test]
fn roaming_consortium_ie_from_beacon_fixture() {
    let mpdu = include_bytes!("fixtures/ieee80211/beacon_rcoi_openroaming.bin");
    let payload = roaming_consortium_ie_payload_from_mgmt_mpdu(mpdu).expect("ie payload");
    assert_eq!(
        rcois_from_ie_payload(&payload),
        "5A03BA0000 BAA2D00000 BAA2D02000"
    );
}
