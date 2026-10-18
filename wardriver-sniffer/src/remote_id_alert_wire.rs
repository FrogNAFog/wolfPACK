//! Build [`wardriver_proto::RemoteIdAlertV1`] from core detector output.

use wardriver_core::remote_id::RemoteIdAlert;
use wardriver_proto::RemoteIdAlertV1;

#[must_use]
pub fn remote_id_alert_to_v1(
    a: &RemoteIdAlert,
    seq: u32,
    sniffer_boot_id: u32,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
) -> RemoteIdAlertV1 {
    let loc = &a.uas_data.location;
    let sys = &a.uas_data.system;
    let op = &a.uas_data.operator_id;
    let basic = a
        .uas_data
        .basic_id
        .iter()
        .zip(a.uas_data.basic_id_valid.iter())
        .find(|(_, v)| **v)
        .map(|(b, _)| b);

    let mut uas_id = [0u8; 20];
    if let Some(id) = a.uas_data.primary_uas_id() {
        uas_id.copy_from_slice(id);
    }
    let lat_e7 = (loc.latitude * 10_000_000.0).round() as i32;
    let lon_e7 = (loc.longitude * 10_000_000.0).round() as i32;
    let op_lat_e7 = (sys.operator_latitude * 10_000_000.0).round() as i32;
    let op_lon_e7 = (sys.operator_longitude * 10_000_000.0).round() as i32;
    let alt_dm = (loc.altitude_geo_m * 10.0)
        .round()
        .clamp(0.0, f32::from(u16::MAX)) as u16;
    let height_agl_dm = (loc.height_m * 10.0)
        .round()
        .clamp(0.0, f32::from(u16::MAX)) as u16;
    let heading = loc.direction_deg.round().clamp(0.0, 359.0) as u16;
    let speed_dmps = (loc.speed_horizontal_mps * 10.0)
        .round()
        .clamp(0.0, f32::from(u16::MAX)) as u16;
    let timestamp_ds = if loc.timestamp_sec >= 0.0 && loc.timestamp_sec < 6553.5 {
        (loc.timestamp_sec * 10.0).round() as u16
    } else {
        0xFFFF
    };

    let mut flags = 0u8;
    if a.uas_data.location_valid {
        flags |= 1;
    }
    if a.uas_data.basic_id_valid.iter().any(|&v| v) {
        flags |= 2;
    }
    if a.uas_data.system_valid {
        flags |= 4;
    }
    if a.uas_data.operator_id_valid {
        flags |= 8;
    }

    let mut operator_id = [0u8; 20];
    if a.uas_data.operator_id_valid {
        operator_id.copy_from_slice(&op.operator_id);
    }

    RemoteIdAlertV1 {
        sniffer_id: a.sniffer_id,
        seq,
        sniffer_boot_id,
        sniffer_scan_end_ms: scan_end_ms,
        sniffer_scan_end_unix_ms: scan_end_unix_ms,
        src: a.src,
        channel: a.channel,
        rssi: a.rssi,
        detection_method: a.method,
        signal_kind: a.signal_kind.as_u8(),
        uas_id,
        drone_lat_e7: lat_e7,
        drone_lon_e7: lon_e7,
        alt_geo_dm: alt_dm,
        heading_deg: heading,
        speed_h_dmps: speed_dmps,
        operator_lat_e7: op_lat_e7,
        operator_lon_e7: op_lon_e7,
        flags,
        id_type: basic.map(|b| b.id_type).unwrap_or(0),
        ua_type: basic.map(|b| b.ua_type).unwrap_or(0),
        status: loc.status,
        height_agl_dm,
        height_type: loc.height_type,
        horiz_accuracy: loc.horiz_accuracy & 0x0F,
        vert_accuracy: loc.vert_accuracy & 0x0F,
        speed_accuracy: loc.speed_accuracy & 0x0F,
        baro_accuracy: loc.baro_accuracy & 0x0F,
        ts_accuracy: loc.ts_accuracy & 0x0F,
        timestamp_ds,
        operator_location_type: sys.operator_location_type,
        operator_id_type: op.operator_id_type,
        operator_id,
    }
}
