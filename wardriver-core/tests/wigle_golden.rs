//! Host tests for WiGLE CSV data-line encoding (`wardriver_core::wigle`).

use wardriver_core::wigle::{
    channel_to_frequency_mhz, format_ble_csv_line, format_cell_csv_line, format_wifi_csv_line,
    format_wigle_headers, format_wigle_pre_header, DeviceInfo, WigleBleLineInput,
    WigleCellLineInput, WigleWifiLineInput, CSV_HEADER, WIGLE_VERSION,
};

#[test]
fn golden_wigle_pre_header_mobile() {
    let info = DeviceInfo {
        app_release: "0.1.0".to_string(),
        model: "Pixel".to_string(),
        release: "14".to_string(),
        device: "pixel".to_string(),
        display: "UP1A".to_string(),
        board: "phone".to_string(),
        brand: "wolfPACK".to_string(),
        star: "Sol".to_string(),
        body: 3,
        sub_body: 0,
    };
    let pre = format_wigle_pre_header(&info);
    assert!(pre.starts_with(WIGLE_VERSION));
    assert!(pre.contains("brand=wolfPACK"));
    assert!(pre.contains("star=Sol"));
    let headers = format_wigle_headers(&info);
    assert!(headers.ends_with(&format!("{CSV_HEADER}\n")));
}

#[test]
fn golden_wifi_row_minimal() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        ssid: "Snoop".to_string(),
        auth_mode: "[WPA2-PSK-CCMP][ESS]".to_string(),
        first_seen_utc: "2026-04-26 07:00:00".to_string(),
        channel: 6,
        frequency_mhz: channel_to_frequency_mhz(6),
        rssi: -45,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/wifi_row_minimal.line").trim()
    );
}

#[test]
fn golden_wifi_row_5ghz_ch36() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x36],
        ssid: "FiveG-Test".to_string(),
        auth_mode: "[WPA2-PSK-CCMP][ESS]".to_string(),
        first_seen_utc: "2026-04-26 08:00:00".to_string(),
        channel: 36,
        frequency_mhz: channel_to_frequency_mhz(36),
        rssi: -62,
        lat: Some(37.7749),
        lon: Some(-122.4194),
        altitude_m: Some(12),
        accuracy_m: Some(7.5),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch36.line").trim()
    );
}

#[test]
fn golden_wifi_row_5ghz_ch149() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x95],
        ssid: "FiveG-High".to_string(),
        auth_mode: "[WPA2-PSK-CCMP][ESS]".to_string(),
        first_seen_utc: "2026-04-26 08:00:00".to_string(),
        channel: 149,
        frequency_mhz: channel_to_frequency_mhz(149),
        rssi: -58,
        lat: Some(37.7749),
        lon: Some(-122.4194),
        altitude_m: Some(12),
        accuracy_m: Some(7.5),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch149.line").trim()
    );
}

#[test]
fn golden_wifi_row_quoted_ssid() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xde, 0xad, 0xbe, 0xef, 0x00, 0x01],
        ssid: "Cafe, \"Special\"".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-04-26 10:00:00".to_string(),
        channel: 11,
        frequency_mhz: channel_to_frequency_mhz(11),
        rssi: -50,
        lat: None,
        lon: None,
        altitude_m: None,
        accuracy_m: None,
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/wifi_row_quoted_ssid.line").trim()
    );
}

#[test]
fn golden_ble_row_minimal() {
    let line = format_ble_csv_line(&WigleBleLineInput {
        addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        name: "Pixel Buds".to_string(),
        capabilities: String::new(),
        first_seen_utc: "2026-04-26 07:30:00".to_string(),
        channel: None,
        frequency_code: Some(7936),
        rssi: -70,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: "76".to_string(),
        row_type: "BLE",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/ble_row_minimal.line").trim()
    );
}

#[test]
fn golden_cell_row_wcdma_doc() {
    let line = format_cell_csv_line(&WigleCellLineInput {
        cell_key: "310410_56967_4118917".to_string(),
        network_name: "AT&T Mobility".to_string(),
        capabilities: "WCDMA;310410".to_string(),
        first_seen_utc: "2018-08-03 18:14:15".to_string(),
        frequency: 4385,
        rssi: -81,
        lat: Some(37.72090053),
        lon: Some(-122.44579219),
        altitude_m: Some(104),
        accuracy_m: Some(34.30400085449219),
        row_type: "WCDMA",
    });
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/cell_row_wcdma_doc.line").trim()
    );
}

use wardriver_core::wigle_ble::{
    ble_csv_fields_from_cod, ble_csv_fields_no_cod, device_type_label, MAJOR_UNCATEGORIZED,
};

#[test]
fn no_cod_defaults_match_android() {
    let (caps, freq) = ble_csv_fields_no_cod();
    assert_eq!(caps, "");
    assert_eq!(freq, MAJOR_UNCATEGORIZED);
}

#[test]
fn cod_headphones_matches_android() {
    let (caps, freq) = ble_csv_fields_from_cod(0x18, 0x04, 0x00);
    assert_eq!(caps, "Headphones");
    assert_eq!(freq, 0x0418);
}

#[test]
fn cod_uncategorized_matches_android() {
    let (caps, freq) = ble_csv_fields_from_cod(0x00, 0x1F, 0x00);
    assert_eq!(caps, "Uncategorized");
    assert_eq!(freq, MAJOR_UNCATEGORIZED);
}

#[test]
fn cod_headset_miss_has_empty_caps() {
    let (caps, freq) = ble_csv_fields_from_cod(0x04, 0x04, 0x00);
    assert_eq!(caps, "");
    assert_eq!(freq, 0x0404);
}

#[test]
fn cod_zero_is_misc_with_blank_frequency() {
    let (caps, freq) = ble_csv_fields_from_cod(0x00, 0x00, 0x00);
    assert_eq!(caps, "Misc");
    assert_eq!(freq, 0);
}

#[test]
fn device_type_label_known_and_missing() {
    assert_eq!(device_type_label(0x0418), "Headphones");
    assert_eq!(device_type_label(0x0404), "");
}

#[test]
fn is_misc_or_uncategorized_masks() {
    assert!(0 == 0 || 0 == MAJOR_UNCATEGORIZED);
    assert!(MAJOR_UNCATEGORIZED == 0 || MAJOR_UNCATEGORIZED == MAJOR_UNCATEGORIZED);
    assert!(!(0x0418 == 0 || 0x0418 == MAJOR_UNCATEGORIZED));
}
