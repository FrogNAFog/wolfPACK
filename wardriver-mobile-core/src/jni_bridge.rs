//! JSON JNI wrappers for GPS timeline, geofence, home zone, bulk ingest, and map helpers.

use crate::gps_timeline::{GpsTimeline, GPS_HIST_RETENTION_MS};
use crate::home_zone_latch::HomeZoneLatch;
use serde_json::{json, Value};
use wardriver_core::liveness::{
    format_sniffer_last_seen_ago, pending_nearby_stale, pending_nearby_visible, short_sniffer_id,
    sniffer_is_quiet_stale, state_label,
};
use wardriver_core::wigle::{format_wigle_headers, DeviceInfo};
use wardriver_packlink::{unpack_frame, unpack_observation_bulk, MsgType};

use crate::enrichment::{
    enrich_ble_entry_with_extras, enrich_cell, enrich_wifi_ap_with_rcois, EnrichOutcome,
};
use crate::enrichment_session::with_session_mut;
use crate::geofence::{GeofenceEvent, GeofenceMonitor, GeofenceSquare};
use crate::jni_enrich::{
    apply_ephemeral_enrich_fields, cell_observation_to_json, observation_to_json, parse_ap_entry,
    parse_ap_rcois, parse_ble_entry, parse_ble_extras, parse_cell_input, unpack_bulk_payload,
};

/// GPS timeline operation JSON API.
#[must_use]
pub fn gps_timeline_op_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let op = req.get("op").and_then(Value::as_str).unwrap_or("");
    let now_ms = req.get("now_ms").and_then(Value::as_i64).unwrap_or(0);

    match op {
        "constants" => json!({
            "ok": true,
            "gps_hist_retention_ms": GPS_HIST_RETENTION_MS,
            "max_fix_timeline_delta_ms": wardriver_core::gps_anchor::MAX_FIX_TIMELINE_DELTA_MS,
        })
        .to_string(),
        "is_valid_fix_time" => {
            let unix_ms = req.get("unix_ms").and_then(Value::as_i64).unwrap_or(0);
            json!({
                "ok": true,
                "valid": GpsTimeline::is_valid_fix_time(unix_ms, now_ms),
            })
            .to_string()
        }
        "is_stale_replay" => {
            let anchor = req
                .get("hdr_scan_end_unix_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            json!({
                "ok": true,
                "stale": GpsTimeline::is_stale_replay(anchor, now_ms),
            })
            .to_string()
        }
        "is_usable" => {
            let acc = req.get("accuracy_m").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            json!({"ok": true, "usable": GpsTimeline::is_usable(acc)}).to_string()
        }
        _ => {
            let mut timeline = parse_timeline(req.get("timeline"));
            match op {
                "append" => {
                    let fix = match req.get("fix") {
                        Some(f) => f,
                        None => return json!({"ok": false, "error": "fix required"}).to_string(),
                    };
                    let unix_ms = fix.get("unix_ms").and_then(Value::as_i64).unwrap_or(0);
                    let lat = fix.get("lat").and_then(Value::as_f64).unwrap_or(0.0);
                    let lon = fix.get("lon").and_then(Value::as_f64).unwrap_or(0.0);
                    let accuracy_m =
                        fix.get("accuracy_m").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    let altitude_m = fix.get("altitude_m").and_then(Value::as_f64);
                    let appended =
                        timeline.append(unix_ms, lat, lon, accuracy_m, altitude_m, now_ms);
                    build_timeline_response(&timeline, appended)
                }
                "prune" => {
                    timeline.prune(now_ms);
                    build_timeline_response(&timeline, true)
                }
                "merge_route" => {
                    if let Some(arr) = req.get("route").and_then(Value::as_array) {
                        let points: Vec<(i64, f64, f64, f32, Option<f64>)> = arr
                            .iter()
                            .filter_map(|item| {
                                Some((
                                    item.get("unix_ms")?.as_i64()?,
                                    item.get("lat")?.as_f64()?,
                                    item.get("lon")?.as_f64()?,
                                    item.get("accuracy_m")?.as_f64()? as f32,
                                    item.get("altitude_m").and_then(Value::as_f64),
                                ))
                            })
                            .collect();
                        timeline.merge_samples(points, now_ms);
                    }
                    build_timeline_response(&timeline, true)
                }
                "to_hist" => build_timeline_response(&timeline, true),
                other => json!({"ok": false, "error": format!("unknown op '{other}'")}).to_string(),
            }
        }
    }
}

fn parse_timeline(v: Option<&Value>) -> GpsTimeline {
    let Some(arr) = v.and_then(Value::as_array) else {
        return GpsTimeline::new();
    };
    let mut tl = GpsTimeline::new();
    for item in arr {
        let unix_ms = item.get("unix_ms").and_then(Value::as_i64).unwrap_or(0);
        let lat = item.get("lat").and_then(Value::as_f64).unwrap_or(0.0);
        let lon = item.get("lon").and_then(Value::as_f64).unwrap_or(0.0);
        let accuracy_m = item
            .get("accuracy_m")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32;
        let altitude_m = item.get("altitude_m").and_then(Value::as_f64);
        tl.append(unix_ms, lat, lon, accuracy_m, altitude_m, unix_ms.max(0));
    }
    tl
}

fn build_timeline_response(timeline: &GpsTimeline, ok_append: bool) -> String {
    let samples: Vec<Value> = timeline
        .to_fix_hist()
        .iter()
        .map(|(_, fix)| {
            let mut obj = json!({
                "unix_ms": fix.utc_unix_ms,
                "lat": fix.lat,
                "lon": fix.lon,
                "accuracy_m": fix.accuracy_m,
            });
            if let Some(alt) = fix.altitude_m {
                obj["altitude_m"] = json!(alt);
            }
            obj
        })
        .collect();
    json!({
        "ok": true,
        "appended": ok_append,
        "gps_usable": timeline.latest_usable(),
        "size": timeline.len(),
        "timeline": samples,
        "gps_hist": samples,
    })
    .to_string()
}

/// Geofence monitor state-machine update.
#[must_use]
pub fn geofence_monitor_update_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let enabled = req.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let lat = req.get("lat").and_then(Value::as_f64).unwrap_or(0.0);
    let lon = req.get("lon").and_then(Value::as_f64).unwrap_or(0.0);
    let square = GeofenceSquare {
        lat_min: req.get("lat_min").and_then(Value::as_f64).unwrap_or(0.0),
        lat_max: req.get("lat_max").and_then(Value::as_f64).unwrap_or(0.0),
        lon_min: req.get("lon_min").and_then(Value::as_f64).unwrap_or(0.0),
        lon_max: req.get("lon_max").and_then(Value::as_f64).unwrap_or(0.0),
    };
    let was_inside = match req.get("was_inside") {
        Some(v) if v.is_null() => None,
        Some(v) => v.as_bool(),
        None => None,
    };
    let mut monitor = GeofenceMonitor {
        square,
        enabled,
        was_inside,
    };
    let event = monitor.update(lat, lon);
    let event_str = match event {
        GeofenceEvent::Entered => "Entered",
        GeofenceEvent::Left => "Left",
        GeofenceEvent::None => "None",
    };
    let inside = if enabled {
        square.contains(lat, lon)
    } else {
        true
    };
    json!({
        "ok": true,
        "event": event_str,
        "was_inside": monitor.was_inside,
        "inside": inside,
    })
    .to_string()
}

/// Home zone latch update.
#[must_use]
pub fn home_zone_latch_update_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let enabled = req.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let center_lat = req.get("center_lat").and_then(Value::as_f64).unwrap_or(0.0);
    let center_lon = req.get("center_lon").and_then(Value::as_f64).unwrap_or(0.0);
    let radius_m = req.get("radius_m").and_then(Value::as_f64).unwrap_or(0.0);
    let lat = req.get("lat").and_then(Value::as_f64);
    let lon = req.get("lon").and_then(Value::as_f64);
    let gps_ok = req.get("gps_ok").and_then(Value::as_bool).unwrap_or(false);
    let now_ms = req.get("now_ms").and_then(Value::as_u64).unwrap_or(0);
    let mut latch = HomeZoneLatch {
        inside_prev: req
            .get("inside_prev")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        outside_gps_ok_since_ms: req
            .get("outside_since_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };
    let suppressed = latch.wardrive_suppressed(
        enabled, center_lat, center_lon, radius_m, lat, lon, gps_ok, now_ms,
    );
    json!({
        "ok": true,
        "suppressed": suppressed,
        "inside_prev": latch.inside_prev,
        "outside_since_ms": latch.outside_gps_ok_since_ms,
    })
    .to_string()
}

/// Stateless home-zone point test.
#[must_use]
pub fn home_zone_contains_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let inside = HomeZoneLatch::contains(
        req.get("center_lat").and_then(Value::as_f64).unwrap_or(0.0),
        req.get("center_lon").and_then(Value::as_f64).unwrap_or(0.0),
        req.get("radius_m").and_then(Value::as_f64).unwrap_or(0.0),
        req.get("lat").and_then(Value::as_f64).unwrap_or(0.0),
        req.get("lon").and_then(Value::as_f64).unwrap_or(0.0),
    );
    json!({"ok": true, "inside": inside}).to_string()
}

/// Ingest one ObservationBulk PackLink frame (wifi + ble) with open enrichment session.
#[must_use]
pub fn ingest_observation_bulk_json(wire: &[u8], context_json: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(context_json) else {
        return json!({"ok": false, "error": "invalid context json"}).to_string();
    };
    let session_id = req.get("session_id").and_then(Value::as_i64).unwrap_or(1);
    let fail_on_invalid = req
        .get("fail_on_invalid")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let now_ms = req.get("now_ms").and_then(Value::as_i64).unwrap_or(0);

    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::ObservationBulk {
        return json!({"ok": false, "error": "expected ObservationBulk"}).to_string();
    }
    let bulk = match unpack_observation_bulk(frame.payload) {
        Ok(b) => b,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    let (hdr_scan_end_ms, hdr_scan_end_unix_ms, aps_json, bles_json, _) =
        unpack_bulk_payload(&bulk);
    let sniffer_id = bulk.sniffer_id;

    if aps_json.is_empty() && bles_json.is_empty() {
        return json!({"ok": true, "stored": [], "dropped": []}).to_string();
    }

    if fail_on_invalid && GpsTimeline::is_stale_replay(hdr_scan_end_unix_ms as i64, now_ms) {
        let count = aps_json.len() + bles_json.len();
        let dropped: Vec<Value> = (0..count)
            .map(|_| json!({"reason": "stale_replay"}))
            .collect();
        return json!({"ok": true, "stored": [], "dropped": dropped}).to_string();
    }

    let mut stored = Vec::new();
    let mut dropped = Vec::new();

    for ap_val in &aps_json {
        let ap = match parse_ap_entry(Some(ap_val)) {
            Ok(a) => a,
            Err(e) => {
                if fail_on_invalid {
                    return json!({"ok": false, "error": e}).to_string();
                }
                dropped.push(json!({"reason": "parse_ap"}));
                continue;
            }
        };
        let mut ctx = req.clone();
        ctx["hdr_scan_end_ms"] = json!(hdr_scan_end_ms);
        ctx["hdr_scan_end_unix_ms"] = json!(hdr_scan_end_unix_ms);
        ctx["sniffer_id"] = json!(sniffer_id);
        let result = with_session_mut(session_id, |state| {
            apply_ephemeral_enrich_fields(state, &ctx)?;
            Ok(enrich_wifi_ap_with_rcois(
                state,
                session_id,
                hdr_scan_end_ms,
                hdr_scan_end_unix_ms,
                sniffer_id,
                &ap,
                &parse_ap_rcois(Some(ap_val)),
            ))
        });
        match result {
            Ok(EnrichOutcome::Stored(obs)) => stored.push(observation_to_json(&obs)),
            Ok(EnrichOutcome::StoredCell(_)) => dropped.push(json!({"reason": "unexpected_cell"})),
            Ok(EnrichOutcome::StoredProbe(_)) => {
                dropped.push(json!({"reason": "unexpected_probe"}))
            }
            Ok(EnrichOutcome::Dropped(reason)) => dropped.push(json!({"reason": reason})),
            Err(e) => {
                if fail_on_invalid {
                    return json!({"ok": false, "error": e}).to_string();
                }
                dropped.push(json!({"reason": e}));
            }
        }
    }

    for ble_val in &bles_json {
        let ble = match parse_ble_entry(Some(ble_val)) {
            Ok(e) => e,
            Err(e) => {
                if fail_on_invalid {
                    return json!({"ok": false, "error": e}).to_string();
                }
                dropped.push(json!({"reason": "parse_ble"}));
                continue;
            }
        };
        let mut ctx = req.clone();
        ctx["hdr_scan_end_ms"] = json!(hdr_scan_end_ms);
        ctx["hdr_scan_end_unix_ms"] = json!(hdr_scan_end_unix_ms);
        ctx["sniffer_id"] = json!(sniffer_id);
        let result = with_session_mut(session_id, |state| {
            apply_ephemeral_enrich_fields(state, &ctx)?;
            Ok(enrich_ble_entry_with_extras(
                state,
                session_id,
                hdr_scan_end_ms,
                hdr_scan_end_unix_ms,
                sniffer_id,
                &ble,
                &parse_ble_extras(Some(ble_val)),
            ))
        });
        match result {
            Ok(EnrichOutcome::Stored(obs)) => stored.push(observation_to_json(&obs)),
            Ok(EnrichOutcome::StoredCell(obs)) => stored.push(cell_observation_to_json(&obs)),
            Ok(EnrichOutcome::StoredProbe(_)) => {
                dropped.push(json!({"reason": "unexpected_probe"}))
            }
            Ok(EnrichOutcome::Dropped(reason)) => dropped.push(json!({"reason": reason})),
            Err(e) => {
                if fail_on_invalid {
                    return json!({"ok": false, "error": e}).to_string();
                }
                dropped.push(json!({"reason": e}));
            }
        }
    }

    json!({"ok": true, "stored": stored, "dropped": dropped}).to_string()
}

/// Batch-enrich phone-sourced cell towers (one JNI call per GPS fix).
#[must_use]
pub fn ingest_cells_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let session_id = req.get("session_id").and_then(Value::as_i64).unwrap_or(1);
    let cells = match req.get("cells").and_then(Value::as_array) {
        Some(a) => a,
        None => return json!({"ok": true, "stored": [], "dropped": []}).to_string(),
    };
    let mut stored = Vec::new();
    let mut dropped = Vec::new();
    for cell_val in cells {
        let mut cell_req = req.clone();
        cell_req["cell"] = cell_val.clone();
        let cell = match parse_cell_input(&cell_req) {
            Ok(c) => c,
            Err(e) => {
                dropped.push(json!({"reason": e}));
                continue;
            }
        };
        let result = with_session_mut(session_id, |state| {
            apply_ephemeral_enrich_fields(state, &req)?;
            Ok(enrich_cell(state, session_id, &cell))
        });
        match result {
            Ok(EnrichOutcome::StoredCell(obs)) => stored.push(cell_observation_to_json(&obs)),
            Ok(EnrichOutcome::Stored(_)) => dropped.push(json!({"reason": "unexpected_wifi"})),
            Ok(EnrichOutcome::StoredProbe(_)) => {
                dropped.push(json!({"reason": "unexpected_probe"}))
            }
            Ok(EnrichOutcome::Dropped(reason)) => dropped.push(json!({"reason": reason})),
            Err(e) => dropped.push(json!({"reason": e})),
        }
    }
    json!({"ok": true, "stored": stored, "dropped": dropped}).to_string()
}

/// WiGLE CSV pre-header + column header.
#[must_use]
pub fn format_wigle_headers_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let info = DeviceInfo {
        app_release: req
            .get("app_release")
            .and_then(Value::as_str)
            .unwrap_or("0.1.0")
            .to_string(),
        model: req
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("Android")
            .to_string(),
        release: req
            .get("release")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        device: req
            .get("device")
            .and_then(Value::as_str)
            .unwrap_or("phone")
            .to_string(),
        display: req
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        board: req
            .get("board")
            .and_then(Value::as_str)
            .unwrap_or("phone")
            .to_string(),
        brand: req
            .get("brand")
            .and_then(Value::as_str)
            .unwrap_or("wolfPACK")
            .to_string(),
        star: req
            .get("star")
            .and_then(Value::as_str)
            .unwrap_or("Sol")
            .to_string(),
        body: req.get("body").and_then(Value::as_u64).unwrap_or(3) as u8,
        sub_body: req.get("sub_body").and_then(Value::as_u64).unwrap_or(0) as u8,
    };
    json!({"ok": true, "headers": format_wigle_headers(&info)}).to_string()
}

/// Sniffer display helper JSON dispatch.
#[must_use]
pub fn sniffer_display_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let op = req.get("op").and_then(Value::as_str).unwrap_or("");
    let gateway_now = req
        .get("gateway_now_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_seen = req.get("last_seen_ms").and_then(Value::as_u64).unwrap_or(0);
    let connected = req
        .get("gateway_connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match op {
        "quiet_stale" => json!({
            "ok": true,
            "value": sniffer_is_quiet_stale(gateway_now, last_seen, connected),
        })
        .to_string(),
        "pending_visible" => json!({
            "ok": true,
            "value": pending_nearby_visible(gateway_now, last_seen, connected),
        })
        .to_string(),
        "pending_stale" => json!({
            "ok": true,
            "value": pending_nearby_stale(gateway_now, last_seen, connected),
        })
        .to_string(),
        "last_seen_ago" => {
            let ls = req.get("last_seen_ms").and_then(Value::as_u64);
            json!({
                "ok": true,
                "value": format_sniffer_last_seen_ago(gateway_now, ls),
            })
            .to_string()
        }
        "state_label" => {
            let state = req.get("state").and_then(Value::as_u64).map(|v| v as u8);
            json!({"ok": true, "value": state_label(state)}).to_string()
        }
        "short_id" => {
            let id = req.get("sniffer_id").and_then(Value::as_u64).unwrap_or(0) as u32;
            json!({"ok": true, "value": short_sniffer_id(id)}).to_string()
        }
        other => json!({"ok": false, "error": format!("unknown op '{other}'")}).to_string(),
    }
}
