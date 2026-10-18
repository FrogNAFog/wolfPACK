//! Host tests for JSON JNI wrappers (`jni_enrich`).

use serde_json::{json, Value};
use wardriver_mobile_core::jni_enrich::{
    enrich_wifi_ap_json, enrichment_session_close_json, enrichment_session_open_json,
    format_wifi_csv_line_json, unpack_packlink_frame_json,
};
use wardriver_packlink::{default_gateway_info, pack_frame, pack_gateway_info, MsgType};

const SESSION_STORES_FIRST: i64 = 100;
const SESSION_DEDUP: i64 = 101;
const SESSION_PARITY: i64 = 102;
const SESSION_CHANNEL_ZERO: i64 = 103;

fn open_test_session(session_id: i64) {
    let _ = enrichment_session_close_json(&json!({"session_id": session_id}).to_string());
    let out = enrichment_session_open_json(&json!({"session_id": session_id}).to_string());
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["ok"], true);
}

fn enrich_request_json(session_id: i64, lat: f64, lon: f64, bssid_suffix: u8) -> String {
    let bssid = format!("AA:BB:CC:DD:EE:{bssid_suffix:02X}");
    json!({
        "session_id": session_id,
        "hdr_scan_end_ms": 50_000,
        "hdr_scan_end_unix_ms": 1_700_000_000_000u64,
        "sniffer_id": 3,
        "trusted_clock": true,
        "gps_usable": true,
        "inside_home_geofence": false,
        "scan_held": false,
        "fast_mode": false,
        "gps_hist": [{"unix_ms": 1_700_000_000_000i64, "lat": lat, "lon": lon, "accuracy_m": 6.0}],
        "denylist_bssids": [],
        "denylist_ssids": [],
        "ap": {
            "bssid": bssid,
            "channel": 6,
            "rssi": -55,
            "auth": 3,
            "ssid": "Test",
            "obs_mono_ms": 50_000
        }
    })
    .to_string()
}

#[test]
fn unpack_packlink_gateway_info_json() {
    let wire = pack_gateway_info(&default_gateway_info());
    let out = unpack_packlink_frame_json(&wire);
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["msg_type"], MsgType::GatewayInfo as u8);
    assert_eq!(v["msg_type_name"], "GatewayInfo");
    assert!(v["payload_hex"].as_str().unwrap().len() >= 56);
}

#[test]
fn unpack_packlink_rejects_truncated() {
    let wire = pack_frame(MsgType::EventNotify, 0, 1, b"x");
    let out = unpack_packlink_frame_json(&wire[..8]);
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["ok"], false);
}

#[test]
fn enrich_wifi_ap_json_stores_first_sighting() {
    open_test_session(SESSION_STORES_FIRST);
    let req = enrich_request_json(SESSION_STORES_FIRST, 37.7749, -122.4194, 0x01);
    let out = enrich_wifi_ap_json(&req);
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["outcome"], "stored");
    assert_eq!(v["next_id"], 2);
    assert_eq!(v["observation"]["ssid"], "Test");
}

#[test]
fn enrich_wifi_ap_json_dedup_within_radius() {
    open_test_session(SESSION_DEDUP);
    // Store first at base coords.
    let req1 = enrich_request_json(SESSION_DEDUP, 37.7749, -122.4194, 0x02);
    let out1 = enrich_wifi_ap_json(&req1);
    let v1: Value = serde_json::from_str(&out1).expect("json");
    assert_eq!(v1["outcome"], "stored");
    // ~30 m north — within 50 m revisit; session MacTable remembers first row.
    let req2 = enrich_request_json(SESSION_DEDUP, 37.77515, -122.4194, 0x02);
    let out2 = enrich_wifi_ap_json(&req2);
    let v: Value = serde_json::from_str(&out2).expect("json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["outcome"], "dropped");
    assert_eq!(v["reason"], "mac_table_dedup");
}

#[test]
fn enrich_wifi_ap_json_direct_rust_parity() {
    use std::collections::VecDeque;

    use wardriver_core::gps_anchor::GpsFix;
    use wardriver_mobile_core::enrichment::{
        enrich_wifi_ap_with_rcois, EnrichOutcome, EnrichmentState,
    };
    use wardriver_proto::{ApEntryV1, AuthMini};

    open_test_session(SESSION_PARITY);
    let unix = 1_700_000_000_000i64;
    let mut state = EnrichmentState::default();
    state.gps_hist = VecDeque::from([(
        unix,
        GpsFix {
            utc_unix_ms: unix,
            lat: 37.7749,
            lon: -122.4194,
            altitude_m: Some(10.0),
            hdop: None,
            accuracy_m: Some(6.0),
            fix_quality: 1,
        },
    )]);
    let mut ssid = [0u8; 32];
    ssid[..4].copy_from_slice(b"Test");
    let ap = ApEntryV1 {
        bssid: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x03],
        channel: 6,
        rssi: -55,
        auth: AuthMini::Wpa2,
        ssid_len: 4,
        ssid,
        obs_mono_ms: 50_000,
    };
    assert!(matches!(
        enrich_wifi_ap_with_rcois(&mut state, 1, 50_000, unix as u64, 3, &ap, ""),
        EnrichOutcome::Stored(_)
    ));

    let req = enrich_request_json(SESSION_PARITY, 37.7749, -122.4194, 0x03);
    let out = enrich_wifi_ap_json(&req);
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["outcome"], "stored");
}

#[test]
fn enrich_wifi_ap_json_drops_channel_zero() {
    open_test_session(SESSION_CHANNEL_ZERO);
    let mut req: Value = serde_json::from_str(&enrich_request_json(
        SESSION_CHANNEL_ZERO,
        37.7749,
        -122.4194,
        0x11,
    ))
    .unwrap();
    req["ap"]["channel"] = json!(0);
    let out = enrich_wifi_ap_json(&req.to_string());
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["outcome"], "dropped");
    assert_eq!(v["reason"], "invalid_channel");
}

#[test]
fn format_wifi_csv_line_json_includes_rcois() {
    let out = format_wifi_csv_line_json(
        &json!({
            "observation": {
                "bssid": "1A:9F:EE:5C:71:C6",
                "ssid": "Scampoodle",
                "auth_mode": "[WPA2-EAP-CCMP][ESS]",
                "observed_unix_ms": 1_533_130_107_000i64,
                "channel": 161,
                "frequency_mhz": 5805,
                "rssi": -43,
                "lat": 37.76578028,
                "lon": -123.45919439,
                "altitude_m": 67,
                "accuracy_m": 3.216,
                "rcois": "5A03BA0000 BAA2D00000 BAA2D02000"
            }
        })
        .to_string(),
    );
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["ok"], true);
    let line = v["line"].as_str().expect("line");
    assert!(line.contains("5A03BA0000 BAA2D00000 BAA2D02000"));
}
