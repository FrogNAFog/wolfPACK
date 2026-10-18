//! WiFi max TX power caps via ESP-IDF `esp_wifi_set_max_tx_power`.
//!
//! Power is in **0.25 dBm** steps; valid range **8–84** (per IDF docs for ESP32 series).
//! Defaults: low `16` (~4 dBm), high `52` (~13 dBm). NVS tuning can be added later.

use esp_idf_sys::esp_wifi_set_max_tx_power;
use log::warn;

/// Minimum allowed IDF value (2 dBm).
pub const TX_POWER_MIN_IDF: i8 = 8;
/// Maximum allowed IDF value (21 dBm).
pub const TX_POWER_MAX_IDF: i8 = 84;

/// Sniffer / control “SoftAP + ESP-NOW only” cap.
pub const DEFAULT_TX_POWER_LOW: i8 = 16;
/// STA upload window / active scan cap (tunable).
pub const DEFAULT_TX_POWER_HIGH: i8 = 52;

/// Project policy: set max TX power once at boot.
///
/// By default we cap to the low value since fleets are typically co-located.
pub fn apply_default_cap_once() {
    apply_fleet_cap();
}

/// SoftAP + ESP-NOW fleet window (co-located default).
pub fn apply_fleet_cap() {
    set_max_tx_power_quarter_dbm(DEFAULT_TX_POWER_LOW);
}

/// STA upload window (higher cap for home AP reach).
pub fn apply_upload_cap() {
    set_max_tx_power_quarter_dbm(DEFAULT_TX_POWER_HIGH);
}

/// Set max TX power (quarter-dBm units). Clamps to IDF range; logs on failure.
pub fn set_max_tx_power_quarter_dbm(power: i8) {
    let p = power.clamp(TX_POWER_MIN_IDF, TX_POWER_MAX_IDF);
    // SAFETY: called after WiFi driver start; IDF API is thread-safe for this setter.
    let rc = unsafe { esp_wifi_set_max_tx_power(p) };
    if rc != 0 {
        warn!("wifi_tx_power: esp_wifi_set_max_tx_power({p}) rc={rc}");
    }
}

// Legacy per-state setters intentionally removed: this project now sets TX power once at boot.
