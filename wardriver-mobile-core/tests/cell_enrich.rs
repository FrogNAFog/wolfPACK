//! Host tests for phone-sourced cell tower enrichment + CSV export.

use wardriver_mobile_core::enrichment::{
    enrich_cell, CellTowerInput, EnrichOutcome, EnrichmentState,
};
use wardriver_mobile_core::enrichment_session::{
    enrichment_session_close, enrichment_session_open,
};
use wardriver_mobile_core::jni_enrich::{enrich_cell_json, format_cell_csv_line_json};

#[test]
fn enrich_cell_stores_and_formats_wigle_row() {
    let mut state = EnrichmentState::default();
    state.gps_usable = true;
    state.trusted_clock = true;
    let cell = CellTowerInput {
        cell_key: "310410_56967_4118917".to_string(),
        network_name: "AT&T Mobility".to_string(),
        capabilities: "WCDMA;310410".to_string(),
        cell_type: "WCDMA",
        frequency: 4385,
        rssi: -81,
        observed_unix_ms: 1_533_234_855_000,
        lat: 37.72090053,
        lon: -122.44579219,
        altitude_m: Some(104),
        accuracy_m: Some(34.304f32),
    };
    let outcome = enrich_cell(&mut state, 1, &cell);
    let EnrichOutcome::StoredCell(obs) = outcome else {
        panic!("expected stored cell");
    };
    let req = serde_json::json!({"observation": {
        "observed_unix_ms": obs.observed_unix_ms,
        "bssid": obs.cell_key,
        "ssid": obs.network_name,
        "auth_mode": obs.capabilities,
        "signal_kind": obs.cell_type,
        "frequency_mhz": obs.frequency,
        "rssi": obs.rssi,
        "lat": obs.lat,
        "lon": obs.lon,
        "altitude_m": obs.altitude_m,
        "accuracy_m": obs.accuracy_m,
    }});
    let out = format_cell_csv_line_json(&req.to_string());
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["ok"], true);
    let line = v["line"].as_str().expect("line");
    assert!(line.contains("310410_56967_4118917"));
    assert!(line.contains(",,"));
    assert!(line.ends_with(",WCDMA"));
}

#[test]
fn enrich_cell_json_session_roundtrip() {
    enrichment_session_close(42);
    enrichment_session_open(42).expect("open");
    let req = serde_json::json!({
        "session_id": 42,
        "gps_usable": true,
        "trusted_clock": true,
        "cell": {
            "cell_key": "310410_1_2",
            "network_name": "Carrier",
            "capabilities": "LTE;310410",
            "cell_type": "LTE",
            "frequency": 1234,
            "rssi": -90,
            "observed_unix_ms": 1_700_000_000_000i64,
            "lat": 37.0,
            "lon": -122.0,
            "accuracy_m": 5.0,
        }
    });
    let out = enrich_cell_json(&req.to_string());
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["outcome"], "stored");
    enrichment_session_close(42);
}
