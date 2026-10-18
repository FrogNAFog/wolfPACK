//! Helpers bridging [`wardriver_proto::RemoteIdAlertV1`] and control stats / PACKcsv.

use wardriver_core::detection::{DetectionSignalKind, RemoteIdWifiDetectionMethod};
use wardriver_proto::RemoteIdAlertV1;
use wardriver_stats as stats;

use crate::csv::{PackRemoteIdCsvRow, REMOTE_ID_ROW_TYPE_BLE, REMOTE_ID_ROW_TYPE_WIFI};

/// Scratch strings for numeric OpenDroneID enum fields in CSV columns.
pub struct RemoteIdCsvScratch<'a> {
    pub uas_id: &'a mut String,
    pub operator_id: &'a mut String,
    pub id_type: &'a mut String,
    pub ua_type: &'a mut String,
    pub status: &'a mut String,
    pub operator_location_type: &'a mut String,
    pub operator_id_type: &'a mut String,
}

#[must_use]
pub fn remote_id_last_from_alert(now_ms: u64, a: &RemoteIdAlertV1) -> stats::RemoteIdLast {
    stats::RemoteIdLast {
        last_seen_ms: now_ms,
        sniffer_id: a.sniffer_id,
        src: a.src,
        channel: a.channel,
        rssi: a.rssi,
        signal_kind: a.signal_kind,
        detection_method: a.detection_method,
        drone_lat_e7: a.drone_lat_e7,
        drone_lon_e7: a.drone_lon_e7,
    }
}

#[must_use]
pub fn format_ascii_field(bytes: &[u8; 20]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[must_use]
pub fn e7_to_deg(e7: i32) -> Option<f64> {
    if e7 == 0 {
        return None;
    }
    Some(e7 as f64 / 10_000_000.0)
}

#[must_use]
pub fn wifi_transport_token(method: u8, signal_kind: u8) -> &'static str {
    if signal_kind != DetectionSignalKind::Wifi.as_u8() {
        return "";
    }
    match method {
        x if x == RemoteIdWifiDetectionMethod::BeaconVendorIe.as_u8() => "beacon",
        x if x == RemoteIdWifiDetectionMethod::NanAction.as_u8() => "nan",
        _ => "",
    }
}

#[must_use]
pub fn row_type_for_alert(signal_kind: u8) -> &'static str {
    if signal_kind == DetectionSignalKind::Ble.as_u8() {
        REMOTE_ID_ROW_TYPE_BLE
    } else {
        REMOTE_ID_ROW_TYPE_WIFI
    }
}

fn timestamp_sec_from_ds(timestamp_ds: u16) -> Option<f32> {
    if timestamp_ds == 0 || timestamp_ds == 0xFFFF {
        None
    } else {
        Some(timestamp_ds as f32 / 10.0)
    }
}

fn write_u8_field(buf: &mut String, value: u8) {
    buf.clear();
    if value > 0 {
        use core::fmt::Write;
        let _ = write!(buf, "{value}");
    }
}

#[must_use]
pub fn pack_remote_id_csv_row<'a>(
    alert: &RemoteIdAlertV1,
    first_seen_utc: &'a str,
    observer_lat: Option<f64>,
    observer_lon: Option<f64>,
    observer_alt_m: Option<i32>,
    observer_acc_m: Option<f32>,
    scratch: &'a mut RemoteIdCsvScratch<'a>,
) -> PackRemoteIdCsvRow<'a> {
    *scratch.uas_id = format_ascii_field(&alert.uas_id);
    *scratch.operator_id = format_ascii_field(&alert.operator_id);
    write_u8_field(scratch.id_type, alert.id_type);
    write_u8_field(scratch.ua_type, alert.ua_type);
    write_u8_field(scratch.status, alert.status);
    write_u8_field(scratch.operator_location_type, alert.operator_location_type);
    write_u8_field(scratch.operator_id_type, alert.operator_id_type);

    let drone_lat = e7_to_deg(alert.drone_lat_e7);
    let drone_lon = e7_to_deg(alert.drone_lon_e7);
    let operator_lat = e7_to_deg(alert.operator_lat_e7);
    let operator_lon = e7_to_deg(alert.operator_lon_e7);
    let alt_geo_m = if alert.alt_geo_dm > 0 {
        Some(alert.alt_geo_dm as f32 / 10.0)
    } else {
        None
    };
    let heading_deg = if alert.heading_deg <= 359 {
        Some(alert.heading_deg)
    } else {
        None
    };
    let speed_h_mps = if alert.speed_h_dmps > 0 {
        Some(alert.speed_h_dmps as f32 / 10.0)
    } else {
        None
    };
    let height_agl_m = if alert.height_agl_dm > 0 {
        Some(alert.height_agl_dm as f32 / 10.0)
    } else {
        None
    };
    PackRemoteIdCsvRow {
        row_type: row_type_for_alert(alert.signal_kind),
        first_seen_utc,
        lat: observer_lat,
        lon: observer_lon,
        altitude_m: observer_alt_m,
        accuracy_m: observer_acc_m,
        mac: alert.src,
        channel: alert.channel,
        frequency_mhz: crate::csv::channel_to_frequency_mhz(alert.channel),
        rssi: alert.rssi,
        uas_id: scratch.uas_id.as_str(),
        id_type: scratch.id_type.as_str(),
        ua_type: scratch.ua_type.as_str(),
        status: scratch.status.as_str(),
        drone_lat,
        drone_lon,
        alt_geo_m,
        heading_deg,
        speed_h_mps,
        operator_id: scratch.operator_id.as_str(),
        operator_lat,
        operator_lon,
        detection_method_id: alert.detection_method,
        signal_kind: alert.signal_kind,
        wifi_transport: wifi_transport_token(alert.detection_method, alert.signal_kind),
        height_agl_m,
        height_type: if alert.height_type > 0 || height_agl_m.is_some() {
            Some(alert.height_type)
        } else {
            None
        },
        horiz_accuracy: if alert.horiz_accuracy > 0 {
            Some(alert.horiz_accuracy)
        } else {
            None
        },
        vert_accuracy: if alert.vert_accuracy > 0 {
            Some(alert.vert_accuracy)
        } else {
            None
        },
        speed_accuracy: if alert.speed_accuracy > 0 {
            Some(alert.speed_accuracy)
        } else {
            None
        },
        baro_accuracy: if alert.baro_accuracy > 0 {
            Some(alert.baro_accuracy)
        } else {
            None
        },
        ts_accuracy: if alert.ts_accuracy > 0 {
            Some(alert.ts_accuracy)
        } else {
            None
        },
        timestamp_sec: timestamp_sec_from_ds(alert.timestamp_ds),
        operator_location_type: if alert.operator_location_type > 0 {
            Some(alert.operator_location_type)
        } else {
            None
        },
        operator_id_type: if alert.operator_id_type > 0 {
            Some(alert.operator_id_type)
        } else {
            None
        },
    }
}
