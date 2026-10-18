//! Dynamic frequency scaling: lower CPU clock when idle (WiFi/BT hold RF locks as needed).

use esp_idf_sys::{esp_pm_config_t, esp_pm_configure, ESP_OK};

/// Apply a conservative DFS profile (no automatic light sleep — avoids surprising WiFi/BLE stalls).
pub fn init() {
    let cfg = esp_pm_config_t {
        max_freq_mhz: 240,
        min_freq_mhz: 80,
        light_sleep_enable: false,
    };
    let rc = unsafe { esp_pm_configure(&cfg as *const _ as *const core::ffi::c_void) };
    if rc != ESP_OK {
        log::warn!("pm: esp_pm_configure rc={rc}");
    } else {
        log::info!("pm: DFS enabled (240 MHz max, 80 MHz min, no light sleep)");
    }
}
