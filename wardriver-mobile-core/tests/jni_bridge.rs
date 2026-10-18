//! Host tests for jni_bridge JSON wrappers.

use serde_json::{json, Value};
use wardriver_mobile_core::enrichment_session::{
    enrichment_session_close, enrichment_session_open,
};
use wardriver_mobile_core::jni_bridge::{
    format_wigle_headers_json, geofence_monitor_update_json, gps_timeline_op_json,
    home_zone_contains_json, home_zone_latch_update_json, ingest_cells_json, sniffer_display_json,
};

#[test]
fn gps_timeline_append_and_prune() {
    let now = 1_700_000_000_000i64;
    let append = json!({
        "op": "append",
        "now_ms": now,
        "timeline": [],
        "fix": {"unix_ms": now, "lat": 37.77, "lon": -122.42, "accuracy_m": 8.0}
    });
    let out: Value = serde_json::from_str(&gps_timeline_op_json(&append.to_string())).unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["size"], 1);

    let stale =
        json!({"op": "is_stale_replay", "now_ms": now, "hdr_scan_end_unix_ms": now - 400_000});
    let stale_v: Value = serde_json::from_str(&gps_timeline_op_json(&stale.to_string())).unwrap();
    assert_eq!(stale_v["stale"], true);
}

#[test]
fn geofence_monitor_left_event() {
    let req = json!({
        "enabled": true,
        "lat_min": 37.0, "lat_max": 38.0,
        "lon_min": -123.0, "lon_max": -122.0,
        "was_inside": true,
        "lat": 36.5, "lon": -122.5
    });
    let out: Value = serde_json::from_str(&geofence_monitor_update_json(&req.to_string())).unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["event"], "Left");
}

#[test]
fn home_zone_latch_entry() {
    let req = json!({
        "enabled": true,
        "center_lat": 37.7749, "center_lon": -122.4194,
        "radius_m": 100.0,
        "inside_prev": false,
        "outside_since_ms": 0,
        "lat": 37.7749, "lon": -122.4194,
        "gps_ok": true,
        "now_ms": 1000
    });
    let out: Value = serde_json::from_str(&home_zone_latch_update_json(&req.to_string())).unwrap();
    assert_eq!(out["suppressed"], true);
}

#[test]
fn home_zone_contains() {
    let req = json!({
        "center_lat": 37.0, "center_lon": -122.0,
        "radius_m": 1000.0,
        "lat": 37.0, "lon": -122.0
    });
    let out: Value = serde_json::from_str(&home_zone_contains_json(&req.to_string())).unwrap();
    assert_eq!(out["inside"], true);
}

#[test]
fn format_wigle_headers_json_ok() {
    let req = json!({"model": "Pixel", "brand": "wolfPACK"});
    let out: Value = serde_json::from_str(&format_wigle_headers_json(&req.to_string())).unwrap();
    assert!(out["headers"].as_str().unwrap().contains("WigleWifi-1.6"));
}

#[test]
fn sniffer_display_quiet_stale() {
    let req = json!({
        "op": "quiet_stale",
        "gateway_now_ms": 30_000,
        "last_seen_ms": 10_000,
        "gateway_connected": true
    });
    let out: Value = serde_json::from_str(&sniffer_display_json(&req.to_string())).unwrap();
    assert_eq!(out["value"], true);
}

#[test]
fn ingest_cells_json_stores() {
    enrichment_session_close(99);
    enrichment_session_open(99).expect("open");
    let req = json!({
        "session_id": 99,
        "trusted_clock": true,
        "gps_usable": true,
        "inside_home_geofence": false,
        "cells": [{
            "cell_key": "310410_1_2",
            "network_name": "Carrier",
            "capabilities": "LTE;310410",
            "cell_type": "LTE",
            "frequency": 2100,
            "rssi": -85,
            "observed_unix_ms": 1_700_000_000_000i64,
            "lat": 37.77,
            "lon": -122.42,
            "accuracy_m": 8.0
        }]
    });
    let out: Value = serde_json::from_str(&ingest_cells_json(&req.to_string())).unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["stored"].as_array().unwrap().len(), 1);
    enrichment_session_close(99);
}
