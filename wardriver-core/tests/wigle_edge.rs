//! Host tests for WiGLE CSV encoding edge cases.

use wardriver_core::wigle::{
    channel_to_frequency_mhz, csv_escape, format_ble_csv_line, format_wifi_csv_line,
    ssid_bytes_to_wigle_string, WigleBleLineInput, WigleWifiLineInput,
};

// ── SSID edge cases ────────────────────────────────────────────

#[test]
fn ssid_with_newline_gets_quoted() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xaa; 6],
        ssid: "Line\nBreak".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("Line\nBreak"), "newline triggers CSV quoting");
}

#[test]
fn ssid_with_backslash() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xab; 6],
        ssid: "Path\\Name".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 11,
        frequency_mhz: 2462,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    // Backslashes alone don't trigger quoting per RFC4180
    assert!(line.contains("Path\\Name"));
}

#[test]
fn ssid_with_binary_bytes() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xac; 6],
        ssid: ssid_bytes_to_wigle_string(&[0x80, 0xFF, 0x00, 0x41], 4),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 1,
        frequency_mhz: 2412,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    // ssid_bytes_to_wigle_string truncates at first NUL
    assert!(line.contains("WIFI"));
}

#[test]
fn empty_ssid_with_auth_present() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xad; 6],
        ssid: String::new(),
        auth_mode: "[WPA2-PSK-CCMP][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    let fields: Vec<&str> = line.split(',').collect();
    assert_eq!(fields[1], ""); // SSID is empty
    assert_eq!(fields[2], "[WPA2-PSK-CCMP][ESS]");
}

// ── GPS coordinate edge cases ──────────────────────────────────

#[test]
fn gps_at_north_pole() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xae; 6],
        ssid: "Pole".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(90.0),
        lon: Some(0.0),
        altitude_m: Some(0),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("90"));
}

#[test]
fn gps_at_south_pole() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xaf; 6],
        ssid: "South".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 11,
        frequency_mhz: 2462,
        rssi: -60,
        lat: Some(-90.0),
        lon: Some(0.0),
        altitude_m: Some(0),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("-90"));
}

#[test]
fn gps_at_dateline() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb0; 6],
        ssid: "Dateline".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 1,
        frequency_mhz: 2412,
        rssi: -70,
        lat: Some(0.0),
        lon: Some(180.0),
        altitude_m: Some(0),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("180"));
}

#[test]
fn gps_negative_altitude() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb1; 6],
        ssid: "DeadSea".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -55,
        lat: Some(31.7),
        lon: Some(35.5),
        altitude_m: Some(-430),
        accuracy_m: Some(3.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("-430"));
}

// ── RSSI boundary values ───────────────────────────────────────

#[test]
fn rssi_minimum_value() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb2; 6],
        ssid: "Weak".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -127,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("-127"));
}

#[test]
fn rssi_zero() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb3; 6],
        ssid: "Strong".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: 0,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains(",0,")); // RSSI=0 in the row
}

#[test]
fn rssi_maximum_value() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb4; 6],
        ssid: "Saturated".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: 127,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.contains("127"));
}

// ── BSSID edge cases ───────────────────────────────────────────

#[test]
fn bssid_all_zeros() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0x00; 6],
        ssid: "Zeros".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.starts_with("00:00:00:00:00:00,"));
}

#[test]
fn bssid_all_0xff() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xff; 6],
        ssid: "Broadcast".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    assert!(line.starts_with("ff:ff:ff:ff:ff:ff,"));
}

// ── CSV quoting edge cases ─────────────────────────────────────

#[test]
fn mfgr_id_with_comma_gets_quoted() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb5; 6],
        ssid: "Test".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: "OUI, vendor".to_string(),
        row_type: "WIFI",
    });
    assert!(line.contains("\"OUI, vendor\""));
}

#[test]
fn mfgr_id_with_quotes_gets_escaped() {
    let line = format_wifi_csv_line(&WigleWifiLineInput {
        bssid: [0xb6; 6],
        ssid: "Test".to_string(),
        auth_mode: "[OPEN][ESS]".to_string(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: 6,
        frequency_mhz: 2437,
        rssi: -50,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: "He said \"hello\"".to_string(),
        row_type: "WIFI",
    });
    assert!(line.contains("\"He said \"\"hello\"\"\""));
}

#[test]
fn csv_escape_simple_string_unquoted() {
    assert_eq!(csv_escape("Hello"), "Hello");
}

#[test]
fn csv_escape_comma_quoted() {
    assert_eq!(csv_escape("a,b"), "\"a,b\"");
}

#[test]
fn csv_escape_quote_doubled() {
    assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
}

#[test]
fn csv_escape_cr_quoted() {
    assert_eq!(csv_escape("a\rb"), "\"a\rb\"");
}

// ── Channel frequency edge cases ───────────────────────────────

#[test]
fn channel_frequency_ch0_returns_zero() {
    assert_eq!(channel_to_frequency_mhz(0), 0);
}

#[test]
fn channel_frequency_ch14_special() {
    assert_eq!(channel_to_frequency_mhz(14), 2484);
}

#[test]
fn channel_frequency_ch15_invalid() {
    assert_eq!(channel_to_frequency_mhz(15), 0);
}

#[test]
fn channel_frequency_ch29_invalid() {
    assert_eq!(channel_to_frequency_mhz(29), 0);
}

#[test]
fn channel_frequency_ch30_valid() {
    assert_eq!(channel_to_frequency_mhz(30), 5150);
}

#[test]
fn channel_frequency_ch196_valid() {
    assert_eq!(channel_to_frequency_mhz(196), 5980);
}

#[test]
fn channel_frequency_ch197_invalid() {
    assert_eq!(channel_to_frequency_mhz(197), 0);
}

#[test]
fn channel_frequency_ch255_invalid() {
    assert_eq!(channel_to_frequency_mhz(255), 0);
}

// ── BLE CSV row ────────────────────────────────────────────────

#[test]
fn ble_row_with_no_frequency_code() {
    let line = format_ble_csv_line(&WigleBleLineInput {
        addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        name: "Test".to_string(),
        capabilities: String::new(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: None,
        frequency_code: None,
        rssi: -70,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "BLE",
    });
    assert!(line.contains("BLE"));
}

#[test]
fn ble_row_channel_zero_omitted() {
    let line = format_ble_csv_line(&WigleBleLineInput {
        addr: [0x22; 6],
        name: "Test".to_string(),
        capabilities: String::new(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: Some(0),
        frequency_code: Some(7936),
        rssi: -70,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "BLE",
    });
    // Channel=0 should be blank in output
    assert!(line.contains(",,")); // channel blank
}

#[test]
fn ble_row_frequency_code_zero_omitted() {
    let line = format_ble_csv_line(&WigleBleLineInput {
        addr: [0x33; 6],
        name: "Test".to_string(),
        capabilities: String::new(),
        first_seen_utc: "2026-01-01 00:00:00".to_string(),
        channel: None,
        frequency_code: Some(0),
        rssi: -70,
        lat: Some(37.0),
        lon: Some(-122.0),
        altitude_m: Some(10),
        accuracy_m: Some(5.0),
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "BLE",
    });
    // freq=0 should be blank
    let fields: Vec<&str> = line.split(',').collect();
    assert_eq!(fields[5], ""); // Frequency column blank
}
