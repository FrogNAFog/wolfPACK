//! Session-scoped MacTable: dedup across enrich calls without per-request `mac_last`.

use serde_json::{json, Value};
use wardriver_mobile_core::jni_enrich::{
    enrich_wifi_ap_json, enrichment_session_close_json, enrichment_session_open_json,
};

fn enrich_wifi_req(session_id: i64, lat: f64, lon: f64, bssid_suffix: u8) -> String {
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
fn dedup_across_two_enrich_calls_without_mac_last() {
    let _ = enrichment_session_close_json(r#"{"session_id":1}"#);
    let open = enrichment_session_open_json(r#"{"session_id":1}"#);
    let open_v: Value = serde_json::from_str(&open).expect("json");
    assert_eq!(open_v["ok"], true);

    // First sighting at base coords.
    let out1 = enrich_wifi_ap_json(&enrich_wifi_req(1, 37.7749, -122.4194, 0x02));
    let v1: Value = serde_json::from_str(&out1).expect("json");
    assert_eq!(v1["outcome"], "stored");

    // ~30 m north, same MAC — within 50 m revisit; session MacTable remembers first row.
    let out2 = enrich_wifi_ap_json(&enrich_wifi_req(1, 37.77515, -122.4194, 0x02));
    let v2: Value = serde_json::from_str(&out2).expect("json");
    assert_eq!(v2["outcome"], "dropped");
    assert_eq!(v2["reason"], "mac_table_dedup");
}

#[test]
fn next_id_increments_in_session() {
    let _ = enrichment_session_close_json(r#"{"session_id":2}"#);
    let _ = enrichment_session_open_json(r#"{"session_id":2}"#);

    let out1 = enrich_wifi_ap_json(&enrich_wifi_req(2, 37.7749, -122.4194, 0x01));
    let v1: Value = serde_json::from_str(&out1).expect("json");
    assert_eq!(v1["next_id"], 2);

    let out2 = enrich_wifi_ap_json(&enrich_wifi_req(2, 37.7749, -122.4194, 0x03));
    let v2: Value = serde_json::from_str(&out2).expect("json");
    assert_eq!(v2["next_id"], 3);
}
