//! WiFi scanning and conversion to WiGLE CSV row fields.
//!
//! This module contains shared scan-related helpers and WiGLE row structs used by the
//! control node when merging sniffer reports.
//!
//! Fleet note: merged rows from sniffers use an empty WiGLE **`mfgr_id`** on the
//! control node (reserved for beacon vendor data). Sniffer identity is exposed
//! on the control dashboard via `/api/stats`, not in the CSV `MfgrId` field.

use std::sync::Arc;

use crate::gps::Fix;

use wardriver_core::gps_anchor::{self, GpsFix};
use wardriver_core::wigle;

/// Minimum fix quality required to record a location.
pub const MIN_FIX_QUALITY: u8 = gps_anchor::MIN_FIX_QUALITY;

/// Maximum HDOP allowed to record a location.
pub const MAX_HDOP: f32 = gps_anchor::MAX_HDOP;

/// One WiGLE-compatible row for Bluetooth Low Energy observations.
#[derive(Clone, Debug)]
pub struct WigleBleRow {
    pub addr: [u8; 6],
    pub name: String,
    /// WiGLE `AuthMode`: Android `DEVICE_TYPE_LEGEND` label.
    pub capabilities: String,
    pub first_seen_utc: Arc<str>,
    /// WiGLE `Channel`: Android leaves BLE channel null (blank in CSV).
    pub channel: Option<u8>,
    /// WiGLE v1.6 Bluetooth rows use `Frequency` as a Bluetooth "Device Type" code (or blank),
    /// not an RF center frequency in MHz. Leave `None` when unknown/zero.
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

#[derive(Clone, Debug)]
pub struct WigleWifiRow {
    pub bssid: [u8; 6],
    pub ssid: String,
    pub auth_mode: String,        // capabilities string
    pub first_seen_utc: Arc<str>, // "YYYY-MM-DD hh:mm:ss" UTC
    pub channel: u8,
    pub frequency_mhz: u16,
    pub rssi: i8,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub rcois: String,
    pub mfgr_id: String,
    pub row_type: &'static str, // "WIFI"
}

pub fn ssid_bytes_to_wigle_string(ssid: &[u8], declared_len: usize) -> String {
    wigle::ssid_bytes_to_wigle_string(ssid, declared_len)
}

pub fn hdop_to_accuracy_m(hdop: f32) -> f32 {
    gps_anchor::hdop_to_accuracy_m(hdop)
}

/// True when the GPS fix is good enough to arm fleet sniffing, apply the home-geo gate,
/// and emit WiGLE rows with lat/lon and [`hdop_to_accuracy_m`] (HDOP required).
pub fn is_fix_usable_for_location(f: &Fix) -> bool {
    gps_anchor::is_fix_usable_for_location(&fix_to_gps_fix(f))
}

fn fix_to_gps_fix(f: &Fix) -> GpsFix {
    GpsFix {
        utc_unix_ms: f.utc_unix_ms,
        lat: f.lat,
        lon: f.lon,
        altitude_m: f.altitude_m,
        hdop: f.hdop,
        accuracy_m: None,
        fix_quality: f.fix_quality,
    }
}
