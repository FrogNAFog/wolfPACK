//! Remote ID alert wire roundtrips.

use wardriver_proto::{
    pack_remote_id_alert, unpack_remote_id_alert, RemoteIdAlertV1, ESP_NOW_MAX_FRAME_LEN,
    PROTO_VERSION, REMOTE_ID_ALERT_BODY_SIZE, REMOTE_ID_ALERT_BODY_WITH_TAIL_SIZE,
    REMOTE_ID_ALERT_PREFIX_SIZE,
};

fn sample_alert() -> RemoteIdAlertV1 {
    let mut uas_id = [0u8; 20];
    uas_id[..8].copy_from_slice(b"FA3ABC12");
    let mut operator_id = [0u8; 20];
    operator_id[..6].copy_from_slice(b"PILOT1");
    RemoteIdAlertV1 {
        sniffer_id: 7,
        seq: 42,
        sniffer_boot_id: 0x1122_3344,
        sniffer_scan_end_ms: 60_000,
        sniffer_scan_end_unix_ms: 1_700_000_000_000,
        src: [0x70, 0xC9, 0x4E, 0x12, 0x34, 0x56],
        channel: 6,
        rssi: -68,
        detection_method: 64,
        signal_kind: 0,
        uas_id,
        drone_lat_e7: 38_627_089,
        drone_lon_e7: -90_200_404,
        alt_geo_dm: 1_200,
        heading_deg: 270,
        speed_h_dmps: 150,
        operator_lat_e7: 38_626_000,
        operator_lon_e7: -90_199_000,
        flags: 0x0F,
        id_type: 1,
        ua_type: 2,
        status: 2,
        height_agl_dm: 450,
        height_type: 0,
        horiz_accuracy: 9,
        vert_accuracy: 8,
        speed_accuracy: 3,
        baro_accuracy: 2,
        ts_accuracy: 5,
        timestamp_ds: 1234,
        operator_location_type: 1,
        operator_id_type: 1,
        operator_id,
    }
}

#[test]
fn remote_id_alert_roundtrip_with_tail() {
    let alert = sample_alert();
    let wire = pack_remote_id_alert(alert);
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire.len(), 10 + REMOTE_ID_ALERT_BODY_WITH_TAIL_SIZE);
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
    let back = unpack_remote_id_alert(&wire).expect("unpack");
    assert_eq!(back, alert);
}

#[test]
fn remote_id_alert_short_body_backward_compat() {
    let alert = sample_alert();
    let mut wire = pack_remote_id_alert(alert);
    wire.truncate(10 + REMOTE_ID_ALERT_PREFIX_SIZE);
    assert_eq!(wire.len(), 10 + REMOTE_ID_ALERT_BODY_SIZE);
    let back = unpack_remote_id_alert(&wire).expect("unpack short");
    assert_eq!(back.sniffer_id, alert.sniffer_id);
    assert_eq!(back.uas_id, alert.uas_id);
    assert_eq!(back.id_type, 0);
    assert_eq!(back.operator_id, [0; 20]);
}
