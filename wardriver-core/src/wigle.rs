//! WiGLE CSV v1.6 data-line encoding (host-testable).
//!
//! Control firmware writes these lines via `writeln!` into the SD queue; this module
//! produces the exact comma-separated row bytes without ESP-IDF or stats dependencies.

use crate::wigle_ble::{self, MAJOR_UNCATEGORIZED};

use wardriver_proto::{ApEntryV1, AuthMini, BleEntryV1};

/// WiGLE CSV format version token.
pub const WIGLE_VERSION: &str = "WigleWifi-1.6";

/// CSV header row for WiGLE v1.6 data lines.
pub const CSV_HEADER: &str = "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type";

/// Device metadata for WiGLE CSV pre-header (mobile + control).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub app_release: String,
    pub model: String,
    pub release: String,
    pub device: String,
    pub display: String,
    pub board: String,
    pub brand: String,
    pub star: String,
    pub body: u8,
    pub sub_body: u8,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            app_release: "0.1.0".to_string(),
            model: "wolfPACK".to_string(),
            release: "esp-idf".to_string(),
            device: "wolfPACK".to_string(),
            display: "headless".to_string(),
            board: "phone".to_string(),
            brand: "wolfPACK".to_string(),
            star: "Sol".to_string(),
            body: 3,
            sub_body: 0,
        }
    }
}

/// WiGLE CSV v1.6 pre-header line (no trailing newline).
#[must_use]
pub fn format_wigle_pre_header(info: &DeviceInfo) -> String {
    let brand = csv_escape(&info.brand);
    format!(
        "{ver},appRelease={app},model={model},release={rel},device={dev},display={disp},board={board},brand={brand},star={star},body={body},subBody={sub}",
        ver = WIGLE_VERSION,
        app = info.app_release,
        model = info.model,
        rel = info.release,
        dev = info.device,
        disp = info.display,
        board = info.board,
        brand = brand,
        star = info.star,
        body = info.body,
        sub = info.sub_body,
    )
}

/// Pre-header + column header + trailing newline for upload file start.
#[must_use]
pub fn format_wigle_headers(info: &DeviceInfo) -> String {
    format!("{}\n{}\n", format_wigle_pre_header(info), CSV_HEADER)
}

/// WiGLE `Frequency` (MHz) from 802.11 channel number; `0` when unknown or invalid.
///
/// 2.4 GHz: ch 1–13 → 2407 + 5×ch; ch 14 → 2484. 5 GHz: ch 30–196 → 5000 + 5×ch.
pub fn channel_to_frequency_mhz(ch: u8) -> u16 {
    if ch == 14 {
        return 2484;
    }
    if (1..=13).contains(&ch) {
        return 2407 + (ch as u16) * 5;
    }
    if ch >= 30 && ch <= 196 {
        return 5000 + (ch as u16) * 5;
    }
    0
}

/// Build a WiGLE CSV–safe SSID string from raw bytes (truncates at first NUL).
pub fn ssid_bytes_to_wigle_string(ssid: &[u8], declared_len: usize) -> String {
    let n = declared_len.min(ssid.len());
    let slice = &ssid[..n];
    let slice = slice.split(|&b| b == 0).next().unwrap_or(&[]);
    String::from_utf8_lossy(slice).into_owned()
}

/// WiGLE WiFi row fields before CSV escaping (control `WigleWifiRow` mirror).
#[derive(Clone, Debug, PartialEq)]
pub struct WigleWifiLineInput {
    pub bssid: [u8; 6],
    pub ssid: String,
    pub auth_mode: String,
    pub first_seen_utc: String,
    pub channel: u8,
    pub frequency_mhz: u16,
    pub rssi: i8,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub rcois: String,
    pub mfgr_id: String,
    pub row_type: &'static str,
}

/// WiGLE BLE row fields before CSV escaping (control `WigleBleRow` mirror).
#[derive(Clone, Debug, PartialEq)]
pub struct WigleBleLineInput {
    pub addr: [u8; 6],
    pub name: String,
    /// WiGLE `AuthMode`: Android `DEVICE_TYPE_LEGEND` label (no `[LE]` suffix).
    pub capabilities: String,
    pub first_seen_utc: String,
    /// WiGLE `Channel`: Android leaves BLE channel null (blank in CSV).
    pub channel: Option<u8>,
    /// Bluetooth device-type code in WiGLE `Frequency` (omit when `0`).
    pub frequency_code: Option<u16>,
    pub rssi: i8,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub rcois: String,
    pub mfgr_id: String,
    pub row_type: &'static str,
}

/// WiGLE cell row fields before CSV escaping (phone cell-tower mirror).
#[derive(Clone, Debug, PartialEq)]
pub struct WigleCellLineInput {
    /// WiGLE `MAC` column: composite cell key (not a WiFi BSSID).
    pub cell_key: String,
    /// WiGLE `SSID` column: operator / network name.
    pub network_name: String,
    /// WiGLE `AuthMode` column: e.g. `WCDMA;310410` or `CDMA;1234`.
    pub capabilities: String,
    pub first_seen_utc: String,
    /// ARFCN / UARFCN / EARFCN / NRARFCN; CDMA uses `0`.
    pub frequency: u32,
    pub rssi: i8,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f64>,
    pub row_type: &'static str,
}

/// Map proto auth enum to WiGLE `AuthMode` capabilities string (always suffixed with `[ESS]`).
pub fn authmini_to_caps(a: AuthMini) -> String {
    let base = match a {
        AuthMini::Open => "[OPEN]".to_string(),
        AuthMini::Wep => "[WEP]".to_string(),
        AuthMini::Wpa => "[WPA-PSK]".to_string(),
        AuthMini::Wpa2 => "[WPA2-PSK-CCMP]".to_string(),
        AuthMini::WpaWpa2 => "[WPA-PSK][WPA2-PSK-CCMP]".to_string(),
        AuthMini::Wpa2Ent => "[WPA2-EAP-CCMP]".to_string(),
        AuthMini::Wpa3 => "[WPA3-SAE]".to_string(),
        AuthMini::Wpa2Wpa3 => "[WPA2-PSK-CCMP][WPA3-SAE]".to_string(),
        AuthMini::Unknown => "[UNKNOWN]".to_string(),
    };
    format!("{base}[ESS]")
}

/// Build WiGLE WiFi data line from a parsed `ApEntryV1` plus ingest-side GPS/timestamp fields.
pub fn ap_entry_to_wifi_line(
    ap: &ApEntryV1,
    first_seen_utc: &str,
    fix_lat: Option<f64>,
    fix_lon: Option<f64>,
    fix_altitude_m: Option<i32>,
    fix_accuracy_m: Option<f32>,
) -> WigleWifiLineInput {
    WigleWifiLineInput {
        bssid: ap.bssid,
        ssid: ssid_bytes_to_wigle_string(&ap.ssid, ap.ssid_len as usize),
        auth_mode: authmini_to_caps(ap.auth),
        first_seen_utc: first_seen_utc.to_string(),
        channel: ap.channel,
        frequency_mhz: channel_to_frequency_mhz(ap.channel),
        rssi: ap.rssi,
        lat: fix_lat,
        lon: fix_lon,
        altitude_m: fix_altitude_m,
        accuracy_m: fix_accuracy_m,
        rcois: String::new(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    }
}

/// Build WiGLE BLE data line from a `BleEntryV1` plus ingest-side GPS/timestamp fields.
pub fn ble_entry_to_ble_line(
    e: &BleEntryV1,
    first_seen_utc: &str,
    fix_lat: Option<f64>,
    fix_lon: Option<f64>,
    fix_altitude_m: Option<i32>,
    fix_accuracy_m: Option<f32>,
) -> WigleBleLineInput {
    let end = e.name.iter().position(|&x| x == 0).unwrap_or(e.name.len());
    let name = String::from_utf8_lossy(&e.name[..end]).trim().to_string();
    let mfgr_id = if e.company_id != 0 {
        e.company_id.to_string()
    } else {
        String::new()
    };
    let (capabilities, frequency_code) = wigle_ble::ble_csv_fields_no_cod();
    WigleBleLineInput {
        addr: e.addr,
        name,
        capabilities,
        first_seen_utc: first_seen_utc.to_string(),
        channel: None,
        frequency_code: Some(frequency_code),
        rssi: e.rssi,
        lat: fix_lat,
        lon: fix_lon,
        altitude_m: fix_altitude_m,
        accuracy_m: fix_accuracy_m,
        rcois: String::new(),
        mfgr_id,
        row_type: "BLE",
    }
}

/// RFC-safe `Option` -> string: `Some(v)` to its `Display`, `None` to empty.
fn opt_to_str<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

/// Format one WiGLE WiFi CSV data row (no trailing newline).
pub fn format_wifi_csv_line(inp: &WigleWifiLineInput) -> String {
    let lat = opt_to_str(inp.lat);
    let lon = opt_to_str(inp.lon);
    let alt = opt_to_str(inp.altitude_m);
    let acc = opt_to_str(inp.accuracy_m);
    format!(
        "{mac},{ssid},{auth},{seen},{ch},{freq},{rssi},{lat},{lon},{alt},{acc},{rcois},{mfgr},{typ}",
        mac = format_bssid(&inp.bssid),
        ssid = csv_escape(&inp.ssid),
        auth = csv_escape(&inp.auth_mode),
        seen = inp.first_seen_utc,
        ch = inp.channel,
        freq = inp.frequency_mhz,
        rssi = inp.rssi,
        lat = lat,
        lon = lon,
        alt = alt,
        acc = acc,
        rcois = csv_escape(&inp.rcois),
        mfgr = csv_escape(&inp.mfgr_id),
        typ = inp.row_type,
    )
}

/// Format one WiGLE BLE CSV data row (no trailing newline).
///
/// Matches WiGLE Android `ObservationUploader`: blank AuthMode/Channel when absent,
/// omit `Frequency` when the device-type code is `0`.
pub fn format_ble_csv_line(inp: &WigleBleLineInput) -> String {
    let lat = opt_to_str(inp.lat);
    let lon = opt_to_str(inp.lon);
    let alt = opt_to_str(inp.altitude_m);
    let acc = opt_to_str(inp.accuracy_m);
    let ch = inp
        .channel
        .filter(|&c| c != 0)
        .map(|v| v.to_string())
        .unwrap_or_default();
    let freq = inp
        .frequency_code
        .filter(|&f| f != 0)
        .map(|v| v.to_string())
        .unwrap_or_default();
    format!(
        "{mac},{ssid},{auth},{seen},{ch},{freq},{rssi},{lat},{lon},{alt},{acc},{rcois},{mfgr},{typ}",
        mac = format_bssid(&inp.addr),
        ssid = csv_escape(&inp.name),
        auth = csv_escape(&inp.capabilities),
        seen = inp.first_seen_utc,
        ch = ch,
        freq = freq,
        rssi = inp.rssi,
        lat = lat,
        lon = lon,
        alt = alt,
        acc = acc,
        rcois = csv_escape(&inp.rcois),
        mfgr = csv_escape(&inp.mfgr_id),
        typ = inp.row_type,
    )
}

/// Default BLE device-type code when no Class of Device AD is present.
pub const BLE_DEFAULT_FREQUENCY_CODE: u16 = MAJOR_UNCATEGORIZED;

/// Format one WiGLE cell CSV data row (no trailing newline).
///
/// Channel is always blank per WiGLE v1.6 cell rows (`seen,,freq` in output).
pub fn format_cell_csv_line(inp: &WigleCellLineInput) -> String {
    let lat = opt_to_str(inp.lat);
    let lon = opt_to_str(inp.lon);
    let alt = opt_to_str(inp.altitude_m);
    let acc = opt_to_str(inp.accuracy_m);
    format!(
        "{key},{name},{caps},{seen},,{freq},{rssi},{lat},{lon},{alt},{acc},,,{typ}",
        key = csv_escape(&inp.cell_key),
        name = csv_escape(&inp.network_name),
        caps = csv_escape(&inp.capabilities),
        seen = inp.first_seen_utc,
        freq = inp.frequency,
        rssi = inp.rssi,
        lat = lat,
        lon = lon,
        alt = alt,
        acc = acc,
        typ = inp.row_type,
    )
}

/// RFC4180: quote fields that contain comma, quote, CR, or LF.
pub fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for ch in s.chars() {
            if ch == '"' {
                out.push('"');
                out.push('"');
            } else {
                out.push(ch);
            }
        }
        out.push('"');
        out
    } else {
        s.to_string()
    }
}

/// Lowercase colon-separated MAC for WiGLE `MAC` column.
pub fn format_bssid(bssid: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5]
    )
}
