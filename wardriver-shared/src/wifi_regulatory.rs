//! WiFi regulatory setup (US country + 5 GHz channel mask). Shared by control and sniffer.

use core::ffi::CStr;

use esp_idf_sys as sys;
use log::{info, warn};

/// Configure US regdomain via IDF country-code table (includes UNII-3 for ch 161).
/// Returns `esp_wifi_set_country_code` rc (`ESP_OK` = 0).
pub fn configure_wifi_country_us() -> i32 {
    unsafe {
        let country = CStr::from_bytes_with_nul(b"US\0").expect("US country literal");
        let rc = sys::esp_wifi_set_country_code(country.as_ptr(), false);
        if rc != 0 {
            warn!("wifi_regulatory: esp_wifi_set_country_code(US) rc={rc}");
        } else {
            let mut cfg = core::mem::MaybeUninit::<sys::wifi_country_t>::uninit();
            if sys::esp_wifi_get_country(cfg.as_mut_ptr()) == 0 {
                let cfg = cfg.assume_init();
                let mask = cfg.wifi_5g_channel_mask;
                info!(
                    "wifi_regulatory: US country ok cc={}{}{} 5g_mask=0x{mask:X}",
                    cfg.cc[0] as char, cfg.cc[1] as char, cfg.cc[2] as char,
                );
            } else {
                info!("wifi_regulatory: US country ok");
            }
        }
        rc
    }
}

/// Dual-band (2.4 + 5 GHz); required before configuring a 5 GHz SoftAP channel.
pub fn set_wifi_band_mode_auto() {
    unsafe {
        let rc = sys::esp_wifi_set_band_mode(sys::wifi_band_mode_t_WIFI_BAND_MODE_AUTO);
        if rc != 0 {
            warn!("wifi_regulatory: esp_wifi_set_band_mode(AUTO) rc={rc}");
        } else {
            info!("wifi_regulatory: band mode AUTO");
        }
    }
}

/// Sniffer: allow 2.4 + 5 GHz channel sets (required for DFS listen on C5).
#[cfg(feature = "sniffer")]
pub fn set_sniffer_band_mode_auto() {
    set_wifi_band_mode_auto();
}
