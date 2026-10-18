//! WiGLE CSV v1.6 writer.
//!
//! This module builds the WiGLE CSV pre-header and header and can append
//! WiFi (and later BLE) rows to a `Write` sink.
//!
//! Spec: https://api.wigle.net/csvFormat.html

use std::io::{Result as IoResult, Write};

use wardriver_core::mac_table::{LaneFlags, MacTable as CoreMacTable, MacTableApplyResult};
use wardriver_core::wigle::{
    self, format_ble_csv_line, format_wifi_csv_line, WigleBleLineInput, WigleWifiLineInput,
};

use crate::wifi_scan::{WigleBleRow, WigleWifiRow};
pub use wardriver_core::mac_table::WigleRowDecision;
pub use wardriver_core::wigle::channel_to_frequency_mhz;
use wardriver_stats as stats;

pub const WIGLE_VERSION: &str = "WigleWifi-1.6";

/// Detection native Flock alert sessions (`*.detectioncsv`).
/// Pack-aligned spec: `Detection-detectioncsv-2.0` is WiGLE v1.6 column prefix + 7 Flock extension columns
/// (no legacy `mode` field). See [`/repos/pack/docs/DEFLOCKCSV.md`].
pub const DETECTIONCSV_FORMAT_TOKEN: &str = "Detection-detectioncsv-2.0";

/// CSV header row for [`DETECTIONCSV_FORMAT_TOKEN`] data lines (must start with the full WiGLE v1.6
/// header — `CSV_HEADER` — then the LPR-camera extension columns).
pub const DETECTIONCSV_HEADER: &str = "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type,detection_method_id,signal_kind,rssi_min,rssi_max,rssi_avg_q8,wildcard_count,distinct_ch";

/// CSV header for WiGLE v1.6.
pub const CSV_HEADER: &str = "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type";

/// `detectioncsv` row-type tokens (column `Type`).
pub const DETECTION_ROW_TYPE_DETECTION_WIFI: &str = "DETECTION-WIFI";
pub const DETECTION_ROW_TYPE_DETECTION_BLE: &str = "DETECTION-BLE";

/// PACK mobile surveillance sessions (`*.mdetectioncsv` under `mobile_surveillance/detections/`).
pub const MDETECTIONCSV_FORMAT_TOKEN: &str = "Detection-mdetectioncsv-1.0";

/// Same WiGLE v1.6 + extension columns as [`DETECTIONCSV_HEADER`] (PACK `mdeflock_csv.rs`).
pub const MDETECTIONCSV_HEADER: &str = DETECTIONCSV_HEADER;

pub const MDETECTION_ROW_TYPE_MOBILE_SURVEILLANCE_BLE: &str = "MOBILE-SURVEILLANCE-BLE";
pub const MDETECTION_ROW_TYPE_MOBILE_SURVEILLANCE_WIFI: &str = "MOBILE-SURVEILLANCE-WIFI";

/// Which SD session queue to use for a detection row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectionSessionKind {
    /// `det-*.detectioncsv` (Flock infrastructure).
    Detection,
    /// `Axon-*.mdetectioncsv` (mobile surveillance).
    Mdetection,
}

/// Row `Type` token and session queue for `method` + RF `signal_kind` (`0` Wi-Fi, `1` BLE).
#[must_use]
pub fn detection_session_for_method(
    method: u8,
    signal_kind: u8,
) -> (&'static str, DetectionSessionKind) {
    use wardriver_core::detection::{
        is_mobile_surveillance_ble_method, is_mobile_surveillance_wifi_method, DetectionSignalKind,
    };
    if signal_kind == DetectionSignalKind::Ble.as_u8() && is_mobile_surveillance_ble_method(method)
    {
        (
            MDETECTION_ROW_TYPE_MOBILE_SURVEILLANCE_BLE,
            DetectionSessionKind::Mdetection,
        )
    } else if signal_kind == DetectionSignalKind::Wifi.as_u8()
        && is_mobile_surveillance_wifi_method(method)
    {
        (
            MDETECTION_ROW_TYPE_MOBILE_SURVEILLANCE_WIFI,
            DetectionSessionKind::Mdetection,
        )
    } else if signal_kind == DetectionSignalKind::Ble.as_u8() {
        (
            DETECTION_ROW_TYPE_DETECTION_BLE,
            DetectionSessionKind::Detection,
        )
    } else {
        (
            DETECTION_ROW_TYPE_DETECTION_WIFI,
            DetectionSessionKind::Detection,
        )
    }
}

/// Encode `row` and enqueue on the correct Detection / mdetection SD session.
pub fn enqueue_detection_session_row(
    sd_io: &crate::storage::SdIoHandle,
    row: &DetectionAlertCsvRow<'_>,
    session: DetectionSessionKind,
) -> bool {
    let mut line = Vec::<u8>::with_capacity(384);
    let wrote = match session {
        DetectionSessionKind::Mdetection => write_mdetection_alert_row(&mut line, row).is_ok(),
        DetectionSessionKind::Detection => write_detection_alert_row(&mut line, row).is_ok(),
    };
    if !wrote {
        return false;
    }
    match session {
        DetectionSessionKind::Mdetection => sd_io.try_enqueue_mdetection_row(line),
        DetectionSessionKind::Detection => sd_io.try_enqueue_detection_row(line),
    }
}

/// OLED title for mobile-surveillance alerts (ASCII).
#[must_use]
pub fn mobile_surveillance_oled_title(method: u8) -> &'static str {
    use wardriver_core::detection::{
        MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID, MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID,
    };
    if method == MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID
        || method == MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID
    {
        "I-PRO"
    } else {
        "AXON"
    }
}
pub const PACKCSV_EXT: &str = "PACKcsv";

pub const PACK_REMOTE_ID_FORMAT_TOKEN: &str = "PACK-remote-id-1.1";

pub const PACK_REMOTE_ID_HEADER: &str = concat!(
    "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type",
    ",uas_id,id_type,ua_type,status,drone_lat,drone_lon,alt_geo_m,heading_deg,speed_h_mps,operator_id,operator_lat,operator_lon,detection_method_id,signal_kind,wifi_transport,height_agl_m,height_type,horiz_accuracy,vert_accuracy,speed_accuracy,baro_accuracy,ts_accuracy,timestamp_sec,operator_location_type,operator_id_type"
);

pub const REMOTE_ID_ROW_TYPE_WIFI: &str = "REMOTE-ID-WIFI";
pub const REMOTE_ID_ROW_TYPE_BLE: &str = "REMOTE-ID-BLE";

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub app_release: String,
    pub model: String,
    pub release: String,
    pub device: String,
    pub display: String,
    pub board: String,
    pub brand: String,
    pub star: String, // "Sol"
    pub body: u8,     // 3 for Earth
    pub sub_body: u8, // 0 unless moon etc.
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            app_release: env!("CARGO_PKG_VERSION").to_string(),
            model: "wolfPACK".to_string(),
            release: "esp-idf".to_string(),
            device: "wolfPACK".to_string(),
            display: "headless".to_string(),
            board: runtime_board_label(),
            brand: "wolfPACK".to_string(),
            star: "Sol".to_string(),
            body: 3,
            sub_body: 0,
        }
    }
}

pub fn write_pre_header<W: Write>(w: &mut W, info: &DeviceInfo) -> IoResult<()> {
    // Pre-header is RFC4180 escaped; we only need quotes around fields with commas.
    // Brand is the most likely to contain commas; keep it safe.
    let brand = wigle::csv_escape(&info.brand);
    writeln!(
        w,
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
        sub = info.sub_body
    )
}

fn runtime_board_label() -> String {
    let target = std::env::var("IDF_TARGET")
        .ok()
        .or_else(|| option_env!("CONFIG_IDF_TARGET").map(str::to_string))
        .unwrap_or_else(|| "esp32".to_string())
        .to_ascii_lowercase();

    match target.as_str() {
        "esp32c5" => "ESP32-C5".to_string(),
        "esp32c6" => "ESP32-C6".to_string(),
        "esp32c3" => "ESP32-C3".to_string(),
        "esp32s3" => "ESP32-S3".to_string(),
        "esp32s2" => "ESP32-S2".to_string(),
        "esp32h2" => "ESP32-H2".to_string(),
        "esp32p4" => "ESP32-P4".to_string(),
        "esp32" => "ESP32".to_string(),
        _ => target.to_ascii_uppercase(),
    }
}

pub fn write_header<W: Write>(w: &mut W) -> IoResult<()> {
    writeln!(w, "{CSV_HEADER}")
}

/// WiGLE v1.6 pre-header plus column header (wardriving CSV; BLE rows use the same schema as WiFi with `Type=BLE`).
pub fn write_wigle_v16_headers<W: Write>(w: &mut W, info: &DeviceInfo) -> IoResult<()> {
    write_pre_header(w, info)?;
    write_header(w)
}

/// Pre-header for Detection `detectioncsv` (line 1). Line 2 is [`DETECTIONCSV_HEADER`].
pub fn write_detection_pre_header<W: Write>(w: &mut W, info: &DeviceInfo) -> IoResult<()> {
    let brand = wigle::csv_escape(&info.brand);
    writeln!(
        w,
        "{fmt},appRelease={app},model={model},release={rel},device={dev},display={disp},board={board},brand={brand},star={star},body={body},subBody={sub}",
        fmt = DETECTIONCSV_FORMAT_TOKEN,
        app = info.app_release,
        model = info.model,
        rel = info.release,
        dev = info.device,
        disp = info.display,
        board = info.board,
        brand = brand,
        star = info.star,
        body = info.body,
        sub = info.sub_body
    )
}

pub fn write_detection_header<W: Write>(w: &mut W) -> IoResult<()> {
    writeln!(w, "{DETECTIONCSV_HEADER}")
}

/// Pre-header for PACK `mdetectioncsv` (line 1). Line 2 is [`MDETECTIONCSV_HEADER`].
pub fn write_mdetection_pre_header<W: Write>(w: &mut W, info: &DeviceInfo) -> IoResult<()> {
    let brand = wigle::csv_escape(&info.brand);
    writeln!(
        w,
        "{fmt},appRelease={app},model={model},release={rel},device={dev},display={disp},board={board},brand={brand},star={star},body={body},subBody={sub}",
        fmt = MDETECTIONCSV_FORMAT_TOKEN,
        app = info.app_release,
        model = info.model,
        rel = info.release,
        dev = info.device,
        disp = info.display,
        board = info.board,
        brand = brand,
        star = info.star,
        body = info.body,
        sub = info.sub_body
    )
}

pub fn write_mdetection_header<W: Write>(w: &mut W) -> IoResult<()> {
    writeln!(w, "{MDETECTIONCSV_HEADER}")
}

pub fn write_pack_remote_id_pre_header<W: Write>(w: &mut W, info: &DeviceInfo) -> IoResult<()> {
    let brand = wigle::csv_escape(&info.brand);
    writeln!(
        w,
        "{fmt},appRelease={app},model={model},release={rel},device={dev},display={disp},board={board},brand={brand},star={star},body={body},subBody={sub}",
        fmt = PACK_REMOTE_ID_FORMAT_TOKEN,
        app = info.app_release,
        model = info.model,
        rel = info.release,
        dev = info.device,
        disp = info.display,
        board = info.board,
        brand = brand,
        star = info.star,
        body = info.body,
        sub = info.sub_body
    )
}

pub fn write_pack_remote_id_header<W: Write>(w: &mut W) -> IoResult<()> {
    writeln!(w, "{PACK_REMOTE_ID_HEADER}")
}

/// One Remote ID `PACKcsv` data row.
#[derive(Clone, Debug)]
pub struct PackRemoteIdCsvRow<'a> {
    pub row_type: &'static str,
    pub first_seen_utc: &'a str,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub mac: [u8; 6],
    pub channel: u8,
    pub frequency_mhz: u16,
    pub rssi: i8,
    pub uas_id: &'a str,
    pub id_type: &'a str,
    pub ua_type: &'a str,
    pub status: &'a str,
    pub drone_lat: Option<f64>,
    pub drone_lon: Option<f64>,
    pub alt_geo_m: Option<f32>,
    pub heading_deg: Option<u16>,
    pub speed_h_mps: Option<f32>,
    pub operator_id: &'a str,
    pub operator_lat: Option<f64>,
    pub operator_lon: Option<f64>,
    pub detection_method_id: u8,
    pub signal_kind: u8,
    pub wifi_transport: &'a str,
    pub height_agl_m: Option<f32>,
    pub height_type: Option<u8>,
    pub horiz_accuracy: Option<u8>,
    pub vert_accuracy: Option<u8>,
    pub speed_accuracy: Option<u8>,
    pub baro_accuracy: Option<u8>,
    pub ts_accuracy: Option<u8>,
    pub timestamp_sec: Option<f32>,
    pub operator_location_type: Option<u8>,
    pub operator_id_type: Option<u8>,
}

pub fn write_pack_remote_id_row<W: Write>(w: &mut W, r: &PackRemoteIdCsvRow<'_>) -> IoResult<()> {
    let lat = r.lat.map(|v| v.to_string()).unwrap_or_default();
    let lon = r.lon.map(|v| v.to_string()).unwrap_or_default();
    let alt = r.altitude_m.map(|v| v.to_string()).unwrap_or_default();
    let acc = r.accuracy_m.map(|v| v.to_string()).unwrap_or_default();
    let drone_lat = r.drone_lat.map(|v| v.to_string()).unwrap_or_default();
    let drone_lon = r.drone_lon.map(|v| v.to_string()).unwrap_or_default();
    let alt_geo = r.alt_geo_m.map(|v| v.to_string()).unwrap_or_default();
    let heading = r.heading_deg.map(|v| v.to_string()).unwrap_or_default();
    let speed_h = r.speed_h_mps.map(|v| v.to_string()).unwrap_or_default();
    let op_lat = r.operator_lat.map(|v| v.to_string()).unwrap_or_default();
    let op_lon = r.operator_lon.map(|v| v.to_string()).unwrap_or_default();
    let height_agl = r.height_agl_m.map(|v| v.to_string()).unwrap_or_default();
    let height_type = r.height_type.map(|v| v.to_string()).unwrap_or_default();
    let horiz_acc = r.horiz_accuracy.map(|v| v.to_string()).unwrap_or_default();
    let vert_acc = r.vert_accuracy.map(|v| v.to_string()).unwrap_or_default();
    let speed_acc = r.speed_accuracy.map(|v| v.to_string()).unwrap_or_default();
    let baro_acc = r.baro_accuracy.map(|v| v.to_string()).unwrap_or_default();
    let ts_acc = r.ts_accuracy.map(|v| v.to_string()).unwrap_or_default();
    let ts_sec = r.timestamp_sec.map(|v| v.to_string()).unwrap_or_default();
    let op_loc_type = r
        .operator_location_type
        .map(|v| v.to_string())
        .unwrap_or_default();
    let op_id_type = r
        .operator_id_type
        .map(|v| v.to_string())
        .unwrap_or_default();
    writeln!(
        w,
        "{mac},,{auth},{seen},{ch},{freq},{rssi},{lat},{lon},{alt},{acc},,{mfgr},{typ},{uas},{idt},{uat},{st},{dlat},{dlon},{altg},{hdg},{spd},{opid},{oplat},{oplon},{dm},{sk},{wtx},{hag},{ht},{ha},{va},{sa},{ba},{ta},{ts},{olt},{oit}",
        mac = wigle::format_bssid(&r.mac),
        auth = "",
        seen = wigle::csv_escape(r.first_seen_utc),
        ch = r.channel,
        freq = r.frequency_mhz,
        rssi = r.rssi,
        lat = lat,
        lon = lon,
        alt = alt,
        acc = acc,
        mfgr = "",
        typ = r.row_type,
        uas = wigle::csv_escape(r.uas_id),
        idt = wigle::csv_escape(r.id_type),
        uat = wigle::csv_escape(r.ua_type),
        st = wigle::csv_escape(r.status),
        dlat = drone_lat,
        dlon = drone_lon,
        altg = alt_geo,
        hdg = heading,
        spd = speed_h,
        opid = wigle::csv_escape(r.operator_id),
        oplat = op_lat,
        oplon = op_lon,
        dm = r.detection_method_id,
        sk = r.signal_kind,
        wtx = wigle::csv_escape(r.wifi_transport),
        hag = height_agl,
        ht = height_type,
        ha = horiz_acc,
        va = vert_acc,
        sa = speed_acc,
        ba = baro_acc,
        ta = ts_acc,
        ts = ts_sec,
        olt = op_loc_type,
        oit = op_id_type,
    )
}

/// One `mdetectioncsv` data row (column layout matches [`DetectionAlertCsvRow`]).
pub fn write_mdetection_alert_row<W: Write>(
    w: &mut W,
    r: &DetectionAlertCsvRow<'_>,
) -> IoResult<()> {
    write_detection_alert_row(w, r)
}

/// One `detectioncsv` v2 data row. Column order matches WiGLE v1.6
/// (`MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type`)
/// followed by the seven LPR-camera extension columns
/// (`detection_method_id,signal_kind,rssi_min,rssi_max,rssi_avg_q8,wildcard_count,distinct_ch`).
/// Pack-aligned with [`/repos/pack/backend/src/deflock_csv.rs`].
#[derive(Clone, Debug)]
pub struct DetectionAlertCsvRow<'a> {
    /// `Type` column token, e.g. [`DETECTION_ROW_TYPE_DETECTION_WIFI`] / [`DETECTION_ROW_TYPE_DETECTION_BLE`].
    pub row_type: &'static str,
    pub detection_method_id: u8,
    pub signal_kind: u8,
    pub first_seen_utc: &'a str,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub mac: [u8; 6],
    /// WiGLE `SSID` column (often empty today).
    pub ssid: &'a str,
    /// WiGLE `AuthMode` column (often empty today).
    pub auth_mode: &'a str,
    /// WiGLE `Channel` column (`0` when unknown — BLE rows always emit `0`).
    pub channel: u8,
    /// WiGLE `Frequency` column in MHz (`0` when unknown — BLE rows always emit `0`).
    pub frequency_mhz: u16,
    pub rssi: i8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub wildcard_count: u16,
    pub distinct_ch: u8,
    /// WiGLE `RCOIs` column (PACK parity; often empty on ESP32).
    pub rcois: &'a str,
    /// WiGLE `MfgrId` column (PACK parity; often empty on ESP32).
    pub mfgr_id: &'a str,
}

pub fn write_detection_alert_row<W: Write>(
    w: &mut W,
    r: &DetectionAlertCsvRow<'_>,
) -> IoResult<()> {
    let lat = r.lat.map(|v| v.to_string()).unwrap_or_default();
    let lon = r.lon.map(|v| v.to_string()).unwrap_or_default();
    let alt = r.altitude_m.map(|v| v.to_string()).unwrap_or_default();
    let acc = r.accuracy_m.map(|v| v.to_string()).unwrap_or_default();
    writeln!(
        w,
        "{mac},{ssid},{auth},{seen},{ch},{freq},{rssi},{lat},{lon},{alt},{acc},{rcois},{mfgr},{typ},{dm},{sk},{rmin},{rmax},{ravg},{wc},{dc}",
        mac = wigle::format_bssid(&r.mac),
        ssid = wigle::csv_escape(r.ssid),
        auth = wigle::csv_escape(r.auth_mode),
        seen = wigle::csv_escape(r.first_seen_utc),
        ch = r.channel,
        freq = r.frequency_mhz,
        rssi = r.rssi,
        lat = lat,
        lon = lon,
        alt = alt,
        acc = acc,
        rcois = wigle::csv_escape(r.rcois),
        mfgr = wigle::csv_escape(r.mfgr_id),
        typ = r.row_type,
        dm = r.detection_method_id,
        sk = r.signal_kind,
        rmin = r.rssi_min,
        rmax = r.rssi_max,
        ravg = r.rssi_avg_q8,
        wc = r.wildcard_count,
        dc = r.distinct_ch,
    )
}

pub fn write_ble_row<W: Write>(w: &mut W, r: &WigleBleRow) -> IoResult<()> {
    let inp = WigleBleLineInput {
        addr: r.addr,
        name: r.name.clone(),
        capabilities: r.capabilities.clone(),
        first_seen_utc: r.first_seen_utc.to_string(),
        channel: r.channel,
        frequency_code: r.frequency_code,
        rssi: r.rssi,
        lat: r.lat,
        lon: r.lon,
        altitude_m: r.altitude_m,
        accuracy_m: r.accuracy_m,
        rcois: r.rcois.clone(),
        mfgr_id: r.mfgr_id.clone(),
        row_type: r.row_type,
    };
    writeln!(w, "{}", format_ble_csv_line(&inp))
}

pub fn write_wifi_row<W: Write>(w: &mut W, r: &WigleWifiRow) -> IoResult<()> {
    let inp = WigleWifiLineInput {
        bssid: r.bssid,
        ssid: r.ssid.clone(),
        auth_mode: r.auth_mode.clone(),
        first_seen_utc: r.first_seen_utc.to_string(),
        channel: r.channel,
        frequency_mhz: r.frequency_mhz,
        rssi: r.rssi,
        lat: r.lat,
        lon: r.lon,
        altitude_m: r.altitude_m,
        accuracy_m: r.accuracy_m,
        rcois: r.rcois.clone(),
        mfgr_id: r.mfgr_id.clone(),
        row_type: r.row_type,
    };
    writeln!(w, "{}", format_wifi_csv_line(&inp))
}

/// Repeat rows are suppressed within this distance of the last logged position (Linux PACK parity).
pub const REVISIT_RADIUS_M: f64 = 50.0;

/// Unified MAC table: WiGLE WiFi/BLE row throttle + LPR-camera follow-up geo lanes.
#[derive(Debug)]
pub struct MacTable {
    inner: CoreMacTable,
}

impl MacTable {
    pub fn new(max_entries: usize, geo_slots_max: usize) -> Self {
        Self {
            inner: CoreMacTable::new(max_entries.max(16), geo_slots_max.max(16)),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn geo_active_count(&self) -> usize {
        self.inner.geo_active_count()
    }

    pub fn remove(&mut self, bssid: &[u8; 6]) -> bool {
        self.inner.remove(bssid)
    }

    pub fn shrink_to_cap_fraction(&mut self, keep_fraction: f64) {
        let evicted = self.inner.shrink_to_cap_fraction(keep_fraction);
        stats::add_mac_table_row_evictions(evicted);
        stats::add_geodeduper_evictions(evicted);
    }

    /// WiGLE WiFi row throttle at `(lat, lon)`; updates table when [`WigleRowDecision::allow_row`].
    pub fn consider_wigle_wifi(
        &mut self,
        bssid: &[u8; 6],
        lat: f64,
        lon: f64,
        rssi: i8,
    ) -> WigleRowDecision {
        let result = self.inner.allow(
            bssid,
            lat,
            lon,
            LaneFlags::WIGLE_WIFI,
            REVISIT_RADIUS_M,
            Some(rssi),
        );
        Self::record_apply_result(&result);
        result.decision
    }

    /// WiGLE BLE row throttle at `(lat, lon)`; updates table when [`WigleRowDecision::allow_row`].
    pub fn consider_wigle_ble(
        &mut self,
        bssid: &[u8; 6],
        lat: f64,
        lon: f64,
        rssi: i8,
    ) -> WigleRowDecision {
        let result = self.inner.allow(
            bssid,
            lat,
            lon,
            LaneFlags::WIGLE_BLE,
            REVISIT_RADIUS_M,
            Some(rssi),
        );
        Self::record_apply_result(&result);
        result.decision
    }

    /// Flock Detection follow-up throttle (`DETECTION_WIFI` or `DETECTION_BLE` lane from alert meta).
    pub fn allow_detection(
        &mut self,
        mac: &[u8; 6],
        lat: f64,
        lon: f64,
        detection_lane: u8,
    ) -> bool {
        let result = self
            .inner
            .allow(mac, lat, lon, detection_lane, REVISIT_RADIUS_M, None);
        Self::record_apply_result(&result);
        result.decision.allow_row
    }

    /// Remote ID PACKcsv follow-up throttle at observer `(lat, lon)`.
    pub fn allow_remote_id(&mut self, mac: &[u8; 6], lat: f64, lon: f64) -> bool {
        let result = self
            .inner
            .allow(mac, lat, lon, LaneFlags::REMOTE_ID, REVISIT_RADIUS_M, None);
        Self::record_apply_result(&result);
        result.decision.allow_row
    }

    fn record_apply_result(result: &MacTableApplyResult) {
        if result.row_evictions > 0 {
            stats::add_mac_table_row_evictions(result.row_evictions);
            stats::add_geodeduper_evictions(result.row_evictions);
        }
        if result.geo_releases > 0 {
            stats::add_mac_table_geo_releases(result.geo_releases);
        }
        if result.decision.allow_row {
            stats::add_geodeduper_allowed(1);
        } else {
            stats::add_geodeduper_suppressed(1);
        }
    }
}

// --- PACK header drift guard (must match Linux PACK `wigle_csv` / `deflock_csv`) ---

const _: () = {
    const fn bytes_equal(a: &str, b: &str) -> bool {
        let ab = a.as_bytes();
        let bb = b.as_bytes();
        if ab.len() != bb.len() {
            return false;
        }
        let mut i = 0usize;
        while i < ab.len() {
            if ab[i] != bb[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    const PACK_WIGLE: &str =
        "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type";
    const PACK_DETECTION: &str =
        "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type,detection_method_id,signal_kind,rssi_min,rssi_max,rssi_avg_q8,wildcard_count,distinct_ch";

    assert!(bytes_equal(CSV_HEADER, PACK_WIGLE));
    assert!(bytes_equal(DETECTIONCSV_HEADER, PACK_DETECTION));
};
