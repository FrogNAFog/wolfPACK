//! Helpers bridging [`wardriver_core::lpr_camera_wifi`] and control stats / CSV.

use wardriver_core::detection::{DetectionSignalKind, DetectionWifiMethod};
use wardriver_core::lpr_camera_ble::{is_axon_oui, is_ipro_oui};
use wardriver_core::lpr_camera_wifi::{mac_matches_lpr_camera_oui, CameraAlertRecord};
use wardriver_proto::DetectionAlertV1;
use wardriver_stats as stats;

/// Map a Wi-Fi `detection_method` ID to its enum. Unknown IDs return `None` (never silently
/// coerce to method 1).
#[must_use]
pub fn wifi_method_from_id(method: u8) -> Option<DetectionWifiMethod> {
    match method {
        1 => Some(DetectionWifiMethod::WildcardProbesOui),
        2 => Some(DetectionWifiMethod::WildcardProbeIeSignatureOui),
        3 => Some(DetectionWifiMethod::WildcardProbeIeSignatureAnyMac),
        4 => Some(DetectionWifiMethod::AxonOui),
        5 => Some(DetectionWifiMethod::IproOui),
        _ => None,
    }
}

#[must_use]
pub fn detection_alert_v1_to_record(a: &DetectionAlertV1) -> Option<CameraAlertRecord> {
    let wifi_method = wifi_method_from_id(a.detection_method)?;
    let oui_matched =
        mac_matches_lpr_camera_oui(&a.src) || is_axon_oui(&a.src) || is_ipro_oui(&a.src);
    Some(CameraAlertRecord {
        sniffer_id: a.sniffer_id,
        src: a.src,
        wildcard_count: u16::from(a.wildcard_count),
        channel: a.channel,
        rssi_min: a.rssi_min,
        rssi_max: a.rssi_max,
        rssi_avg_q8: a.rssi_avg_q8,
        oui_matched,
        signal_kind: DetectionSignalKind::Wifi,
        wifi_method,
    })
}

#[must_use]
pub fn detection_last_from_detection_alert(
    now_ms: u64,
    a: &DetectionAlertV1,
) -> stats::DetectionLast {
    let oui_matched =
        mac_matches_lpr_camera_oui(&a.src) || is_axon_oui(&a.src) || is_ipro_oui(&a.src);
    stats::DetectionLast {
        last_seen_ms: now_ms,
        sniffer_id: a.sniffer_id,
        src: a.src,
        wildcard_count: u16::from(a.wildcard_count),
        channel: a.channel,
        rssi_min: a.rssi_min,
        rssi_max: a.rssi_max,
        rssi_avg_q8: a.rssi_avg_q8,
        oui_matched: u8::from(oui_matched),
        signal_kind: DetectionSignalKind::Wifi.as_u8(),
        detection_method: a.detection_method,
        ble_company_id: 0,
    }
}
