//! Control management SoftAP RF: country/band prep and AP protocol tuning (ESP32-C5).

use anyhow::Result;
use log::{info, warn};

/// US country + (5 GHz only) band AUTO before SoftAP `set_configuration`.
pub fn prepare_control_softap_before_config() -> Result<()> {
    wardriver_shared::wifi_ap_tune::prepare_softap_before_config(cfg!(feature = "mgmt-ap-5ghz"))
        .map_err(|e| anyhow::anyhow!("control: {e}"))
}

/// AP protocols without 802.11b (call after `wifi.start()`).
pub fn configure_control_ap_protocols() {
    let result = wardriver_shared::wifi_ap_tune::configure_ap_protocols();
    if result.set_protocols_rc != 0 {
        warn!(
            "wifi_ap_tune: esp_wifi_set_protocols(AP) rc={}",
            result.set_protocols_rc
        );
    }
    if result.disable_11b_rc != 0 {
        warn!(
            "wifi_ap_tune: esp_wifi_config_11b_rate(AP, disable) rc={}",
            result.disable_11b_rc
        );
    } else {
        info!("wifi_ap_tune: AP protocols g/n/ax + a/n/ac/ax; 11b disabled");
    }
}
