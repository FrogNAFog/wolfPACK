//! ESP-IDF SoftAP RF preparation and protocol tuning shared by control roles.

use anyhow::{anyhow, Result};
use esp_idf_sys::{
    esp_wifi_config_11b_rate, esp_wifi_set_bandwidths, esp_wifi_set_protocols,
    wifi_bandwidth_t_WIFI_BW20, wifi_bandwidths_t, wifi_interface_t_WIFI_IF_AP,
    wifi_protocols_t, ESP_OK, WIFI_PROTOCOL_11A, WIFI_PROTOCOL_11AC, WIFI_PROTOCOL_11AX,
    WIFI_PROTOCOL_11G, WIFI_PROTOCOL_11N,
};

/// Return codes from applying the common SoftAP protocol policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoftapProtocolTuneResult {
    pub set_protocols_rc: i32,
    pub disable_11b_rc: i32,
    pub bandwidth_rc: i32,
}

/// Configure the US country and optionally select automatic 2.4/5 GHz band mode.
pub fn prepare_softap_before_config(enable_band_auto: bool) -> Result<()> {
    if enable_band_auto {
        crate::wifi_regulatory::set_wifi_band_mode_auto();
    }
    let rc = crate::wifi_regulatory::configure_wifi_country_us();
    if rc != ESP_OK as i32 {
        return Err(anyhow!("esp_wifi_set_country_code(US) failed rc={rc}"));
    }
    Ok(())
}

/// Apply the common AP protocol set and disable legacy 802.11b rates.
#[must_use]
pub fn configure_ap_protocols() -> SoftapProtocolTuneResult {
    unsafe {
        let mut protocols = wifi_protocols_t {
            ghz_2g: (WIFI_PROTOCOL_11G | WIFI_PROTOCOL_11N | WIFI_PROTOCOL_11AX) as u16,
            ghz_5g: (WIFI_PROTOCOL_11A
                | WIFI_PROTOCOL_11N
                | WIFI_PROTOCOL_11AC
                | WIFI_PROTOCOL_11AX) as u16,
        };
        SoftapProtocolTuneResult {
            set_protocols_rc: esp_wifi_set_protocols(wifi_interface_t_WIFI_IF_AP, &mut protocols),
            disable_11b_rc: esp_wifi_config_11b_rate(wifi_interface_t_WIFI_IF_AP, true),
            // Pin the AP to 20 MHz so ESP-NOW fleet traffic (peer rate HE20) does not
            // get swept into a 40 MHz AP context when a client joins at 40D: HE40
            // broadcasts are undecodable by the 20 MHz-tuned sniffers.
            // Band mode is AUTO (2.4G + 5G), so the band-split `set_bandwidths` API is
            // required; plain esp_wifi_config_bandwidth returns ESP_ERR_NOT_SUPPORTED.
            bandwidth_rc: {
                let mut bandwidths: wifi_bandwidths_t = core::mem::zeroed();
                bandwidths.ghz_2g = wifi_bandwidth_t_WIFI_BW20;
                bandwidths.ghz_5g = wifi_bandwidth_t_WIFI_BW20;
                esp_wifi_set_bandwidths(wifi_interface_t_WIFI_IF_AP, &mut bandwidths)
            },
        }
    }
}
