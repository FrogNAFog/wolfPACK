//! JSON wrappers for JNI and host tests — PackLink unpack + WiFi enrichment.
//!
//! Kotlin passes UTF-8 JSON strings; responses are JSON objects with `ok` + fields or `error`.

use serde_json::{json, Value};
use wardriver_core::gps_anchor::{self, GpsFix};
use wardriver_core::wigle::{format_ble_csv_line, format_cell_csv_line, format_wifi_csv_line};
use wardriver_packlink::{
    band_mode, capture_job_id, pack_capture_cmd, pack_channel_plan_sync, pack_policy_sync_seq,
    pack_time_sync, unpack_capture_status, unpack_channel_plan_sync_ack, unpack_event_notify,
    unpack_frame, unpack_observation_bulk, unpack_policy_sync_ack, unpack_sniffer_status_batch,
    unpack_spool_replay, unpack_time_sync, CaptureCmdV1, ChannelPlanSyncV1, MsgType, PolicySyncV1,
    RosterEntryV1, SnifferStatusV1, TimeSyncV1,
};
use wardriver_proto::{
    unpack_ble_scan_frag_with_extras, unpack_detection_alert, unpack_detection_ble_alert,
    unpack_remote_id_alert, unpack_scan_frag_with_extras, unpack_ssid_watch_alert, ApEntryV1,
    AuthMini, BleEntryV1, ProbeObservationWire, ScanEntryExtras, CHANNELS_MAX,
    PROBE_OBSERVATION_WIRE_SIZE,
};

use crate::enrichment::{
    enrich_ble_entry_with_extras, enrich_cell, enrich_probe_observation, enrich_wifi_ap_with_rcois,
    CellTowerInput, EnrichOutcome, EnrichmentState, REVISIT_RADIUS_M,
};
use crate::enrichment_session::{
    enrichment_session_close, enrichment_session_hydrate_cell_last,
    enrichment_session_hydrate_mac_last, enrichment_session_open, with_session_mut,
};
use crate::observation::{CellObservation, Observation, ProbeObservation, SignalKind};

/// Unpack a PackLink wire frame; returns JSON with `msg_type`, `flags`, `seq`, `payload_hex`.
#[must_use]
pub fn unpack_packlink_frame_json(wire: &[u8]) -> String {
    match unpack_frame(wire) {
        Ok(frame) => json!({
            "ok": true,
            "msg_type": frame.msg_type as u8,
            "msg_type_name": msg_type_name(frame.msg_type),
            "flags": frame.flags,
            "seq": frame.seq,
            "payload_len": frame.payload.len(),
            "payload_hex": hex_encode(frame.payload),
        })
        .to_string(),
        Err(e) => json!({
            "ok": false,
            "error": e.to_string(),
        })
        .to_string(),
    }
}

/// Enrich one WiFi AP from a JSON request; returns JSON outcome + updated `next_id`.
///
/// Requires a prior [`enrichment_session_open_json`] for `session_id`.
/// Per-call fields: gates, `gps_hist`, denylists, scan header, `ap`. MacTable lives in the session.
#[must_use]
pub fn enrich_wifi_ap_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let session_id = req.get("session_id").and_then(Value::as_i64).unwrap_or(1);
    let hdr_scan_end_ms = req
        .get("hdr_scan_end_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let hdr_scan_end_unix_ms = req
        .get("hdr_scan_end_unix_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let sniffer_id = req.get("sniffer_id").and_then(Value::as_u64).unwrap_or(0) as u32;
    let ap = match parse_ap_entry(req.get("ap")) {
        Ok(a) => a,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let rcois = parse_ap_rcois(req.get("ap"));
    let enrich_result = with_session_mut(session_id, |state| {
        apply_ephemeral_enrich_fields(state, &req)?;
        let outcome = enrich_wifi_ap_with_rcois(
            state,
            session_id,
            hdr_scan_end_ms,
            hdr_scan_end_unix_ms,
            sniffer_id,
            &ap,
            &rcois,
        );
        Ok((outcome, state.next_id))
    });
    enrich_outcome_to_json(enrich_result)
}

/// Enrich one BLE device from a JSON request; returns JSON outcome + updated `next_id`.
///
/// Request shape mirrors [`enrich_wifi_ap_json`] but uses a `ble` object:
/// `addr`, `rssi`, `addr_type`, `company_id`, `name`, `obs_mono_ms`.
#[must_use]
pub fn enrich_ble_ap_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let session_id = req.get("session_id").and_then(Value::as_i64).unwrap_or(1);
    let hdr_scan_end_ms = req
        .get("hdr_scan_end_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let hdr_scan_end_unix_ms = req
        .get("hdr_scan_end_unix_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let sniffer_id = req.get("sniffer_id").and_then(Value::as_u64).unwrap_or(0) as u32;
    let ble = match parse_ble_entry(req.get("ble")) {
        Ok(e) => e,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let extras = parse_ble_extras(req.get("ble"));
    let enrich_result = with_session_mut(session_id, |state| {
        apply_ephemeral_enrich_fields(state, &req)?;
        let outcome = enrich_ble_entry_with_extras(
            state,
            session_id,
            hdr_scan_end_ms,
            hdr_scan_end_unix_ms,
            sniffer_id,
            &ble,
            &extras,
        );
        Ok((outcome, state.next_id))
    });
    enrich_outcome_to_json(enrich_result)
}

/// Enrich one phone-sourced cell tower from a JSON request.
#[must_use]
pub fn enrich_cell_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let session_id = req.get("session_id").and_then(Value::as_i64).unwrap_or(1);
    let cell = match parse_cell_input(&req) {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let enrich_result = with_session_mut(session_id, |state| {
        apply_ephemeral_enrich_fields(state, &req)?;
        let outcome = enrich_cell(state, session_id, &cell);
        Ok((outcome, state.next_id))
    });
    enrich_outcome_to_json(enrich_result)
}

/// Open a wardrive enrichment session (empty MacTable). Optional `mac_last` / `cell_last` seed for tests.
#[must_use]
pub fn enrichment_session_open_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let session_id = match req.get("session_id").and_then(Value::as_i64) {
        Some(id) => id,
        None => return json!({"ok": false, "error": "session_id required"}).to_string(),
    };
    if let Err(e) = enrichment_session_open(session_id) {
        return json!({"ok": false, "error": e}).to_string();
    }
    if let Some(mac_last) = req.get("mac_last") {
        if let Err(e) = enrichment_session_hydrate_mac_last(session_id, mac_last) {
            enrichment_session_close(session_id);
            return json!({"ok": false, "error": e}).to_string();
        }
    }
    if let Some(cell_last) = req.get("cell_last") {
        if let Err(e) = enrichment_session_hydrate_cell_last(session_id, cell_last) {
            enrichment_session_close(session_id);
            return json!({"ok": false, "error": e}).to_string();
        }
    }
    json!({"ok": true, "session_id": session_id}).to_string()
}

/// Close a wardrive enrichment session (idempotent).
pub fn enrichment_session_close_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let session_id = match req.get("session_id").and_then(Value::as_i64) {
        Some(id) => id,
        None => return json!({"ok": false, "error": "session_id required"}).to_string(),
    };
    enrichment_session_close(session_id);
    json!({"ok": true, "session_id": session_id}).to_string()
}

fn enrich_outcome_to_json(enrich_result: Result<(EnrichOutcome, i64), String>) -> String {
    match enrich_result {
        Ok((EnrichOutcome::Stored(obs), next_id)) => json!({
            "ok": true,
            "outcome": "stored",
            "next_id": next_id,
            "revisit_radius_m": REVISIT_RADIUS_M,
            "observation": observation_to_json(&obs),
        })
        .to_string(),
        Ok((EnrichOutcome::StoredCell(obs), next_id)) => json!({
            "ok": true,
            "outcome": "stored",
            "next_id": next_id,
            "revisit_radius_m": REVISIT_RADIUS_M,
            "observation": cell_observation_to_json(&obs),
        })
        .to_string(),
        Ok((EnrichOutcome::StoredProbe(obs), next_id)) => json!({
            "ok": true,
            "outcome": "stored_probe",
            "next_id": next_id,
            "revisit_radius_m": REVISIT_RADIUS_M,
            "observation": probe_observation_to_json(&obs),
        })
        .to_string(),
        Ok((EnrichOutcome::Dropped(reason), next_id)) => json!({
            "ok": true,
            "outcome": "dropped",
            "reason": reason,
            "next_id": next_id,
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e}).to_string(),
    }
}

/// Unpack an `ObservationBulk` PackLink frame; returns JSON with scan header + AP list when parseable.
#[must_use]
pub fn unpack_observation_bulk_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => {
            return json!({"ok": false, "error": e.to_string()}).to_string();
        }
    };
    if frame.msg_type != MsgType::ObservationBulk {
        return json!({
            "ok": false,
            "error": format!("expected ObservationBulk, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    let bulk = match unpack_observation_bulk(frame.payload) {
        Ok(b) => b,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    // Gateway packs either a WiFi scan frag or a BLE scan frag in `payload`, never both.
    let (hdr_scan_end_ms, hdr_scan_end_unix_ms, aps_json, bles_json, scan_parse_error) =
        unpack_bulk_payload(&bulk);
    let mut out = json!({
        "ok": true,
        "flags": frame.flags,
        "seq": frame.seq,
        "batch_id": bulk.batch_id,
        "sniffer_id": bulk.sniffer_id,
        "scan_end_unix_ms": bulk.scan_end_unix_ms,
        "wifi_ap_count": bulk.wifi_ap_count,
        "ble_count": bulk.ble_count,
        "hdr_scan_end_ms": hdr_scan_end_ms,
        "hdr_scan_end_unix_ms": hdr_scan_end_unix_ms,
        "aps": aps_json,
        "bles": bles_json,
    });
    if let Some(err) = scan_parse_error {
        out["scan_parse_error"] = json!(err);
    }
    out.to_string()
}

/// Pack `PolicySyncV1` from JSON request; returns packed PackLink wire bytes as hex.
///
/// Optional `seq` (u32) sets the PackLink frame sequence for PolicySyncAck correlation.
#[must_use]
pub fn pack_policy_sync_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let policy = match parse_policy_sync(&req) {
        Ok(p) => p,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let seq = req.get("seq").and_then(Value::as_u64).unwrap_or(0) as u32;
    let wire = pack_policy_sync_seq(&policy, seq);
    json!({
        "ok": true,
        "wire_hex": hex_encode(&wire),
        "wire_len": wire.len(),
        "seq": seq,
    })
    .to_string()
}

/// Unpack a `SnifferStatus` PackLink frame; returns JSON with `gateway_mono_ms` + `rows`.
#[must_use]
pub fn unpack_sniffer_status_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => {
            return json!({"ok": false, "error": e.to_string()}).to_string();
        }
    };
    if frame.msg_type != MsgType::SnifferStatus {
        return json!({
            "ok": false,
            "error": format!("expected SnifferStatus, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    match unpack_sniffer_status_batch(frame.payload) {
        Ok((gateway_mono_ms, rows)) => {
            let row_json: Vec<Value> = rows.iter().map(sniffer_status_to_json).collect();
            json!({
                "ok": true,
                "flags": frame.flags,
                "seq": frame.seq,
                "gateway_mono_ms": gateway_mono_ms,
                "count": rows.len(),
                "rows": row_json,
            })
            .to_string()
        }
        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
    }
}

/// Pack `ChannelPlanSyncV1` from JSON request; returns packed PackLink wire bytes as hex.
///
/// Optional `seq` (u32) sets the PackLink frame sequence for ChannelPlanSyncAck correlation.
#[must_use]
pub fn pack_channel_plan_sync_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let plan = match parse_channel_plan_sync(&req) {
        Ok(p) => p,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let seq = req.get("seq").and_then(Value::as_u64).unwrap_or(0) as u32;
    let wire = pack_channel_plan_sync(&plan, seq);
    json!({
        "ok": true,
        "wire_hex": hex_encode(&wire),
        "wire_len": wire.len(),
        "seq": seq,
    })
    .to_string()
}

/// Unpack a `ChannelPlanSyncAck` PackLink frame; returns JSON ack fields.
#[must_use]
pub fn unpack_channel_plan_sync_ack_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::ChannelPlanSyncAck {
        return json!({
            "ok": false,
            "error": format!("expected ChannelPlanSyncAck, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    match unpack_channel_plan_sync_ack(frame.payload) {
        Ok(ack) => json!({
            "ok": true,
            "flags": frame.flags,
            "seq": frame.seq,
            "plan_seq": ack.plan_seq,
            "revision": ack.revision,
            "result": ack.result,
            "error_code": ack.error_code,
            "hop_cycle_ms": ack.hop_cycle_ms,
            "unassigned_count": ack.unassigned_count,
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
    }
}

/// Unpack a `PolicySyncAck` PackLink frame; returns JSON with `policy_seq` / `result` / `error_code`.
#[must_use]
pub fn unpack_policy_sync_ack_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::PolicySyncAck {
        return json!({
            "ok": false,
            "error": format!("expected PolicySyncAck, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    match unpack_policy_sync_ack(frame.payload) {
        Ok(ack) => json!({
            "ok": true,
            "flags": frame.flags,
            "seq": frame.seq,
            "policy_seq": ack.policy_seq,
            "result": ack.result,
            "error_code": ack.error_code,
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
    }
}

/// Pack `CaptureCmdV1` for BLE write to gateway `control_cmd` characteristic.
///
/// Request JSON: `{"job_id":1,"duration_sec":60,"seq":0}` (`job_id` defaults to wifi_probe_pcap).
#[must_use]
pub fn pack_capture_cmd_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let cmd = CaptureCmdV1 {
        job_id: req
            .get("job_id")
            .and_then(Value::as_u64)
            .unwrap_or(u64::from(capture_job_id::WIFI_PROBE_PCAP)) as u8,
        duration_sec: req
            .get("duration_sec")
            .and_then(Value::as_u64)
            .unwrap_or(60) as u16,
    };
    let seq = req.get("seq").and_then(Value::as_u64).unwrap_or(0) as u32;
    let wire = pack_capture_cmd(&cmd, seq);
    json!({
        "ok": true,
        "wire_hex": hex_encode(&wire),
        "wire_len": wire.len(),
    })
    .to_string()
}

/// Unpack a `CaptureStatus` PackLink frame (full wire bytes from `event_notify`).
#[must_use]
pub fn unpack_capture_status_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::CaptureStatus {
        return json!({
            "ok": false,
            "error": format!("expected CaptureStatus, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    match unpack_capture_status(frame.payload) {
        Ok(st) => json!({
            "ok": true,
            "active": st.active,
            "hold": st.hold,
            "remaining_ms": st.remaining_ms,
            "frags_rx": st.frags_rx,
            "bytes_written": st.bytes_written,
            "error_code": st.error_code,
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
    }
}

/// Unpack a `SpoolReplay` PackLink frame; returns chunk metadata + payload hex.
#[must_use]
pub fn unpack_spool_replay_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::SpoolReplay {
        return json!({
            "ok": false,
            "error": format!("expected SpoolReplay, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    match unpack_spool_replay(frame.payload) {
        Ok(chunk) => json!({
            "ok": true,
            "spool_offset": chunk.spool_offset,
            "chunk_total": chunk.chunk_total,
            "chunk_index": chunk.chunk_index,
            "payload_hex": hex_encode(&chunk.payload),
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
    }
}

/// Unpack and enrich a `ProbeObservationBulk` PackLink frame.
#[must_use]
pub fn probe_observation_bulk_json(wire: &[u8], session_id: i64) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::ProbeObservationBulk {
        return json!({
            "ok": false,
            "error": format!("expected ProbeObservationBulk, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    let bulk = match wardriver_packlink::unpack_probe_observation_bulk(&frame.payload) {
        Ok(b) => b,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    // Slice payload into ProbeObservationWire entries (41 bytes each)
    let n = bulk.payload.len() / PROBE_OBSERVATION_WIRE_SIZE;
    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * PROBE_OBSERVATION_WIRE_SIZE;
        let slice = &bulk.payload[off..off + PROBE_OBSERVATION_WIRE_SIZE];
        let mut src_mac = [0u8; 6];
        src_mac.copy_from_slice(&slice[0..6]);
        let ssid_len = slice[6];
        let mut ssid = [0u8; 32];
        ssid.copy_from_slice(&slice[7..39]);
        let channel = slice[39];
        let rssi = slice[40] as i8;
        entries.push(ProbeObservationWire {
            src_mac,
            ssid_len,
            ssid,
            channel,
            rssi,
        });
    }
    let enrich_result = with_session_mut(session_id, |state| {
        let mut observations = Vec::new();
        for wire_entry in &entries {
            let outcome = enrich_probe_observation(
                state,
                session_id,
                bulk.scan_end_ms,
                bulk.scan_end_unix_ms,
                bulk.sniffer_id,
                wire_entry,
            );
            if let EnrichOutcome::StoredProbe(obs) = outcome {
                observations.push(obs);
            }
        }
        Ok((observations, state.next_id))
    });
    match enrich_result {
        Ok((observations, next_id)) => {
            let obs_json: Vec<Value> = observations
                .iter()
                .map(|o| probe_observation_to_json(o))
                .collect();
            json!({
                "ok": true,
                "flags": frame.flags,
                "seq": frame.seq,
                "next_id": next_id,
                "observations": obs_json,
                "count": observations.len(),
            })
            .to_string()
        }
        Err(e) => json!({"ok": false, "error": e}).to_string(),
    }
}

/// Pack `TimeSyncV1` from JSON; returns packed PackLink wire bytes as hex.
#[must_use]
pub fn pack_time_sync_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let ts = TimeSyncV1 {
        phone_unix_ms: req
            .get("phone_unix_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        phone_mono_ms: req
            .get("phone_mono_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        gateway_mono_ms: req
            .get("gateway_mono_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        time_set_seq: req.get("time_set_seq").and_then(Value::as_u64).unwrap_or(0) as u32,
    };
    let seq = req.get("seq").and_then(Value::as_u64).unwrap_or(0) as u32;
    let wire = pack_time_sync(&ts, seq);
    json!({
        "ok": true,
        "wire_hex": hex_encode(&wire),
        "wire_len": wire.len(),
    })
    .to_string()
}

/// Unpack a `TimeSync` PackLink frame.
#[must_use]
pub fn unpack_time_sync_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::TimeSync {
        return json!({
            "ok": false,
            "error": format!("expected TimeSync, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    match unpack_time_sync(frame.payload) {
        Ok(ts) => json!({
            "ok": true,
            "phone_unix_ms": ts.phone_unix_ms,
            "phone_mono_ms": ts.phone_mono_ms,
            "gateway_mono_ms": ts.gateway_mono_ms,
            "time_set_seq": ts.time_set_seq,
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
    }
}

/// Unpack `EventNotify` PackLink frame; parses nested wolfPACK alerts for kinds 1-3.
#[must_use]
pub fn unpack_event_notify_json(wire: &[u8]) -> String {
    let frame = match unpack_frame(wire) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    if frame.msg_type != MsgType::EventNotify {
        return json!({
            "ok": false,
            "error": format!("expected EventNotify, got {}", msg_type_name(frame.msg_type)),
        })
        .to_string();
    }
    let ev = match unpack_event_notify(frame.payload) {
        Ok(e) => e,
        Err(e) => return json!({"ok": false, "error": e.to_string()}).to_string(),
    };
    match parse_event_notify_detail(&ev) {
        Ok(detail) => json!({
            "ok": true,
            "event_kind": ev.event_kind,
            "kind": detail.kind,
            "sniffer_id": detail.sniffer_id,
            "detection_method_id": detail.detection_method_id,
            "unix_ms": ev.unix_ms,
            "detail": detail.detail,
            "lat": detail.lat,
            "lon": detail.lon,
        })
        .to_string(),
        Err(e) => json!({"ok": false, "error": e}).to_string(),
    }
}

struct EventNotifyDetail {
    kind: String,
    sniffer_id: u32,
    detection_method_id: u8,
    detail: String,
    lat: Option<f64>,
    lon: Option<f64>,
}

fn parse_event_notify_detail(
    ev: &wardriver_packlink::EventNotifyV1,
) -> Result<EventNotifyDetail, String> {
    use wardriver_packlink::event_kind;
    match ev.event_kind {
        event_kind::DETECTION_WIFI => {
            let alert = unpack_detection_alert(&ev.payload).map_err(|e| e.to_string())?;
            Ok(EventNotifyDetail {
                kind: "DETECTION_WIFI".to_string(),
                sniffer_id: alert.sniffer_id,
                detection_method_id: alert.detection_method,
                detail: format!(
                    "mac={} ch={} rssi_avg={} method={}",
                    bssid_to_colon_hex(&alert.src),
                    alert.channel,
                    alert.rssi_avg_q8 as f32 / 256.0,
                    alert.detection_method
                ),
                lat: None,
                lon: None,
            })
        }
        event_kind::DETECTION_BLE => {
            let alert = unpack_detection_ble_alert(&ev.payload).map_err(|e| e.to_string())?;
            Ok(EventNotifyDetail {
                kind: "DETECTION_BLE".to_string(),
                sniffer_id: alert.sniffer_id,
                detection_method_id: alert.detection_method,
                detail: format!(
                    "mac={} company=0x{:04X} rssi={} method={}",
                    bssid_to_colon_hex(&alert.src),
                    alert.company_id,
                    alert.rssi,
                    alert.detection_method
                ),
                lat: None,
                lon: None,
            })
        }
        event_kind::SSID_WATCH => {
            let alert = unpack_ssid_watch_alert(&ev.payload).map_err(|e| e.to_string())?;
            Ok(EventNotifyDetail {
                kind: "SSID_WATCH".to_string(),
                sniffer_id: alert.sniffer_id,
                detection_method_id: 0,
                detail: format!(
                    "ssid={} bssid={} ch={} rssi={}",
                    alert.ssid,
                    bssid_to_colon_hex(&alert.bssid),
                    alert.channel,
                    alert.rssi
                ),
                lat: None,
                lon: None,
            })
        }
        event_kind::REMOTE_ID => {
            let alert = unpack_remote_id_alert(&ev.payload).map_err(|e| e.to_string())?;
            let lat = f64::from(alert.drone_lat_e7) / 10_000_000.0;
            let lon = f64::from(alert.drone_lon_e7) / 10_000_000.0;
            let (lat, lon) = if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
                (Some(lat), Some(lon))
            } else {
                (None, None)
            };
            Ok(EventNotifyDetail {
                kind: "REMOTE_ID".to_string(),
                sniffer_id: alert.sniffer_id,
                detection_method_id: alert.detection_method,
                detail: remote_id_alert_detail_json(&alert),
                lat,
                lon,
            })
        }
        _ => Err(format!("unsupported event_kind {}", ev.event_kind)),
    }
}

/// Extract complete PackLink frames from a concatenated spool buffer.
pub fn extract_packlink_frames(buffer: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    const HEADER_LEN: usize = 14;
    const CRC_LEN: usize = 4;
    let mut frames = Vec::new();
    let mut offset = 0usize;
    while offset + HEADER_LEN + CRC_LEN <= buffer.len() {
        if buffer[offset..offset + 4] != wardriver_packlink::MAGIC {
            offset += 1;
            continue;
        }
        let payload_len = u16::from_le_bytes([buffer[offset + 8], buffer[offset + 9]]) as usize;
        let frame_len = HEADER_LEN + payload_len + CRC_LEN;
        if offset + frame_len > buffer.len() {
            break;
        }
        let slice = &buffer[offset..offset + frame_len];
        if unpack_frame(slice).is_ok() {
            frames.push(slice.to_vec());
            offset += frame_len;
        } else {
            offset += 1;
        }
    }
    (frames, buffer[offset..].to_vec())
}

fn sniffer_status_to_json(row: &SnifferStatusV1) -> Value {
    let mut obj = json!({
        "sniffer_id": row.sniffer_id,
        "mac": bssid_to_colon_hex(&row.mac),
        "last_seen_ms": row.last_seen_ms,
        "state": row.state,
        "role": row.role,
        "cpu_load_pct": row.cpu_load_pct,
        "heap_avail_pct": row.heap_avail_pct,
        "fleet_slot": row.fleet_slot,
        "reboot_count": row.reboot_count,
        "adoption_state": row.adoption_state,
    });
    if let Some(ext) = row.plan_ext {
        obj["five_ghz_capable"] = json!(ext.five_ghz_capable);
        obj["assigned_band"] = json!(ext.assigned_band);
        obj["pinned_channel"] = json!(ext.pinned_channel);
        obj["assigned_chan_count"] = json!(ext.assigned_chan_count);
        obj["assigned_channels_preview"] = json!(ext.assigned_channels_preview);
    }
    if let Some(ext) = row.adopt_ext {
        obj["adopt_progress"] = json!(ext.adopt_progress);
        obj["adopt_reason"] = json!(ext.adopt_reason);
    }
    obj
}

/// Format one WiGLE WiFi CSV data line from a stored observation JSON object.
#[must_use]
pub fn format_wifi_csv_line_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let Some(obs) = req.get("observation") else {
        return json!({"ok": false, "error": "observation object required"}).to_string();
    };
    let observed_unix_ms = obs
        .get("observed_unix_ms")
        .and_then(Value::as_i64)
        .ok_or("observed_unix_ms")
        .map_err(|e| e.to_string());
    let observed_unix_ms = match observed_unix_ms {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let Some(ts) = gps_anchor::utc_string_from_unix_ms(observed_unix_ms) else {
        return json!({"ok": false, "error": "bad observed_unix_ms"}).to_string();
    };
    let bssid = match parse_bssid(obs.get("bssid")) {
        Ok(b) => b,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let channel = obs.get("channel").and_then(Value::as_u64).unwrap_or(0) as u8;
    let rssi = obs.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i8;
    let ssid = obs
        .get("ssid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let auth_mode = obs
        .get("auth_mode")
        .and_then(Value::as_str)
        .unwrap_or("[UNKNOWN][ESS]")
        .to_string();
    let lat = obs.get("lat").and_then(Value::as_f64);
    let lon = obs.get("lon").and_then(Value::as_f64);
    let altitude_m = obs
        .get("altitude_m")
        .and_then(Value::as_i64)
        .map(|v| v as i32);
    let accuracy_m = obs
        .get("accuracy_m")
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let frequency_mhz = obs
        .get("frequency_mhz")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u16;
    let line = format_wifi_csv_line(&wardriver_core::wigle::WigleWifiLineInput {
        bssid,
        ssid,
        auth_mode,
        first_seen_utc: ts,
        channel,
        frequency_mhz,
        rssi,
        lat,
        lon,
        altitude_m: altitude_m,
        accuracy_m: accuracy_m,
        rcois: obs
            .get("rcois")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        mfgr_id: String::new(),
        row_type: "WIFI",
    });
    json!({"ok": true, "line": line}).to_string()
}

/// Format one WiGLE BLE CSV data line from a stored observation JSON object.
///
/// For BLE rows, `auth_mode` holds the decimal Bluetooth company ID (WiGLE `MfgrId`).
#[must_use]
pub fn format_ble_csv_line_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let Some(obs) = req.get("observation") else {
        return json!({"ok": false, "error": "observation object required"}).to_string();
    };
    let observed_unix_ms = obs
        .get("observed_unix_ms")
        .and_then(Value::as_i64)
        .ok_or("observed_unix_ms")
        .map_err(|e| e.to_string());
    let observed_unix_ms = match observed_unix_ms {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let Some(ts) = gps_anchor::utc_string_from_unix_ms(observed_unix_ms) else {
        return json!({"ok": false, "error": "bad observed_unix_ms"}).to_string();
    };
    let addr = match parse_bssid(obs.get("bssid")) {
        Ok(b) => b,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let rssi = obs.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i8;
    let name = obs
        .get("ssid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let lat = obs.get("lat").and_then(Value::as_f64);
    let lon = obs.get("lon").and_then(Value::as_f64);
    let altitude_m = obs
        .get("altitude_m")
        .and_then(Value::as_i64)
        .map(|v| v as i32);
    let accuracy_m = obs
        .get("accuracy_m")
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    // BLE ingest stores WiGLE `MfgrId` in `auth_mode` (decimal company ID).
    let mfgr_id = obs
        .get("auth_mode")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let ble_capabilities = obs
        .get("ble_capabilities")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let frequency_code = obs
        .get("frequency_mhz")
        .and_then(Value::as_u64)
        .map(|v| v as u16)
        .filter(|&v| v != 0)
        .or(Some(wardriver_core::wigle::BLE_DEFAULT_FREQUENCY_CODE));
    let line = format_ble_csv_line(&wardriver_core::wigle::WigleBleLineInput {
        addr,
        name,
        capabilities: ble_capabilities,
        first_seen_utc: ts,
        channel: None,
        frequency_code,
        rssi,
        lat,
        lon,
        altitude_m,
        accuracy_m,
        rcois: obs
            .get("rcois")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        mfgr_id,
        row_type: "BLE",
    });
    json!({"ok": true, "line": line}).to_string()
}

/// Format one WiGLE cell CSV data line from a stored observation JSON object.
#[must_use]
pub fn format_cell_csv_line_json(input: &str) -> String {
    let Ok(req) = serde_json::from_str::<Value>(input) else {
        return json!({"ok": false, "error": "invalid json"}).to_string();
    };
    let Some(obs) = req.get("observation") else {
        return json!({"ok": false, "error": "observation object required"}).to_string();
    };
    let observed_unix_ms = obs
        .get("observed_unix_ms")
        .and_then(Value::as_i64)
        .ok_or("observed_unix_ms")
        .map_err(|e| e.to_string());
    let observed_unix_ms = match observed_unix_ms {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": e}).to_string(),
    };
    let Some(ts) = gps_anchor::utc_string_from_unix_ms(observed_unix_ms) else {
        return json!({"ok": false, "error": "bad observed_unix_ms"}).to_string();
    };
    let cell_key = obs
        .get("bssid")
        .or_else(|| obs.get("cell_key"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let network_name = obs
        .get("ssid")
        .or_else(|| obs.get("network_name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let capabilities = obs
        .get("auth_mode")
        .or_else(|| obs.get("capabilities"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cell_type = obs
        .get("signal_kind")
        .or_else(|| obs.get("cell_type"))
        .and_then(Value::as_str)
        .unwrap_or("LTE");
    let frequency = obs
        .get("frequency_mhz")
        .or_else(|| obs.get("frequency"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let rssi = obs.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i8;
    let lat = obs.get("lat").and_then(Value::as_f64);
    let lon = obs.get("lon").and_then(Value::as_f64);
    let altitude_m = obs
        .get("altitude_m")
        .and_then(Value::as_i64)
        .map(|v| v as i32);
    let accuracy_m = obs.get("accuracy_m").and_then(Value::as_f64);
    let line = format_cell_csv_line(&wardriver_core::wigle::WigleCellLineInput {
        cell_key,
        network_name,
        capabilities,
        first_seen_utc: ts,
        frequency,
        rssi,
        lat,
        lon,
        altitude_m,
        accuracy_m,
        row_type: cell_type_static(cell_type),
    });
    json!({"ok": true, "line": line}).to_string()
}

fn cell_type_static(s: &str) -> &'static str {
    match s.to_ascii_uppercase().as_str() {
        "GSM" => "GSM",
        "WCDMA" => "WCDMA",
        "LTE" => "LTE",
        "NR" => "NR",
        "CDMA" => "CDMA",
        _ => "LTE",
    }
}

pub(crate) fn parse_cell_input(req: &Value) -> Result<CellTowerInput, String> {
    let cell = req.get("cell").unwrap_or(req);
    let cell_key = cell
        .get("cell_key")
        .or_else(|| cell.get("bssid"))
        .and_then(Value::as_str)
        .ok_or("cell.cell_key required")?
        .to_string();
    let network_name = cell
        .get("network_name")
        .or_else(|| cell.get("ssid"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let capabilities = cell
        .get("capabilities")
        .or_else(|| cell.get("auth_mode"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let type_str = cell
        .get("cell_type")
        .or_else(|| cell.get("signal_kind"))
        .and_then(Value::as_str)
        .unwrap_or("LTE");
    let cell_type = cell_type_static(type_str);
    let frequency = cell
        .get("frequency")
        .or_else(|| cell.get("frequency_mhz"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let rssi = cell.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i8;
    let observed_unix_ms = cell
        .get("observed_unix_ms")
        .and_then(Value::as_i64)
        .ok_or("cell.observed_unix_ms")?;
    let lat = cell.get("lat").and_then(Value::as_f64).ok_or("cell.lat")?;
    let lon = cell.get("lon").and_then(Value::as_f64).ok_or("cell.lon")?;
    let altitude_m = cell
        .get("altitude_m")
        .and_then(Value::as_i64)
        .map(|v| v as i32);
    let accuracy_m = cell
        .get("accuracy_m")
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    Ok(CellTowerInput {
        cell_key,
        network_name,
        capabilities,
        cell_type,
        frequency,
        rssi,
        observed_unix_ms,
        lat,
        lon,
        altitude_m,
        accuracy_m,
    })
}

/// Decode bulk `payload` into WiFi APs and/or BLE devices for Kotlin ingest.
pub(crate) fn unpack_bulk_payload(
    bulk: &wardriver_packlink::ObservationBulkV1,
) -> (u32, u64, Vec<Value>, Vec<Value>, Option<String>) {
    if bulk.wifi_ap_count > 0 {
        match unpack_scan_frag_with_extras(&bulk.payload) {
            Ok((hdr, aps, extras)) => {
                let ap_json: Vec<Value> = aps
                    .iter()
                    .zip(extras.iter())
                    .map(|(ap, ex)| ap_entry_to_json(ap, ex))
                    .collect();
                return (
                    hdr.sniffer_scan_end_ms,
                    hdr.sniffer_scan_end_unix_ms,
                    ap_json,
                    Vec::new(),
                    None,
                );
            }
            Err(e) => {
                return (
                    0,
                    bulk.scan_end_unix_ms,
                    Vec::new(),
                    Vec::new(),
                    Some(e.to_string()),
                );
            }
        }
    }
    if bulk.ble_count > 0 {
        match unpack_ble_scan_frag_with_extras(&bulk.payload) {
            Ok((hdr, entries, extras)) => {
                let ble_json: Vec<Value> = entries
                    .iter()
                    .zip(extras.iter())
                    .map(|(e, ex)| ble_entry_to_json(e, ex))
                    .collect();
                return (
                    hdr.sniffer_scan_end_ms,
                    hdr.sniffer_scan_end_unix_ms,
                    Vec::new(),
                    ble_json,
                    None,
                );
            }
            Err(e) => {
                return (
                    0,
                    bulk.scan_end_unix_ms,
                    Vec::new(),
                    Vec::new(),
                    Some(e.to_string()),
                );
            }
        }
    }
    (0, bulk.scan_end_unix_ms, Vec::new(), Vec::new(), None)
}

pub(crate) fn observation_to_json(obs: &crate::observation::Observation) -> Value {
    json!({
        "id": obs.id,
        "session_id": obs.session_id,
        "sniffer_id": obs.sniffer_id,
        "observed_unix_ms": obs.observed_unix_ms,
        "lat": obs.lat,
        "lon": obs.lon,
        "altitude_m": obs.altitude_m,
        "accuracy_m": obs.accuracy_m,
        "signal_kind": obs.signal_kind.as_str(),
        "bssid": bssid_to_colon_hex(&obs.bssid),
        "ssid": obs.ssid,
        "channel": obs.channel,
        "frequency_mhz": obs.frequency_mhz,
        "rssi": obs.rssi,
        "auth_mode": obs.auth_mode,
        "rcois": obs.rcois,
        "ble_capabilities": obs.ble_capabilities,
    })
}

pub(crate) fn cell_observation_to_json(obs: &crate::observation::CellObservation) -> Value {
    json!({
        "id": obs.id,
        "session_id": obs.session_id,
        "sniffer_id": 0,
        "observed_unix_ms": obs.observed_unix_ms,
        "lat": obs.lat,
        "lon": obs.lon,
        "altitude_m": obs.altitude_m,
        "accuracy_m": obs.accuracy_m,
        "signal_kind": obs.cell_type,
        "bssid": obs.cell_key,
        "ssid": obs.network_name,
        "channel": 0,
        "frequency_mhz": obs.frequency,
        "rssi": obs.rssi,
        "auth_mode": obs.capabilities,
    })
}

pub(crate) fn probe_observation_to_json(obs: &ProbeObservation) -> Value {
    json!({
        "id": obs.id,
        "session_id": obs.session_id,
        "sniffer_id": obs.sniffer_id,
        "observed_unix_ms": obs.observed_unix_ms,
        "lat": obs.lat,
        "lon": obs.lon,
        "altitude_m": obs.altitude_m,
        "accuracy_m": obs.accuracy_m,
        "signal_kind": "PROBE_REQ",
        "src_mac": bssid_to_colon_hex(&obs.src_mac),
        "ssid": obs.ssid,
        "channel": obs.channel,
        "rssi": obs.rssi,
    })
}

fn ap_entry_to_json(ap: &ApEntryV1, extras: &ScanEntryExtras) -> Value {
    json!({
        "bssid": bssid_to_colon_hex(&ap.bssid),
        "channel": ap.channel,
        "rssi": ap.rssi,
        "auth": auth_to_u8(ap.auth),
        "ssid": ssid_bytes_to_wigle_string(&ap.ssid, ap.ssid_len as usize),
        "obs_mono_ms": ap.obs_mono_ms,
        "rcois": wardriver_core::wigle_rcoi::rcois_from_ie_payload(&extras.rcoi_ie_payload),
    })
}

fn ble_entry_to_json(e: &BleEntryV1, extras: &ScanEntryExtras) -> Value {
    let end = e.name.iter().position(|&x| x == 0).unwrap_or(e.name.len());
    let name = String::from_utf8_lossy(&e.name[..end]).trim().to_string();
    let ble_cod = extras.ble_cod.map(|c| json!([c[0], c[1], c[2]]));
    json!({
        "addr": bssid_to_colon_hex(&e.addr),
        "rssi": e.rssi,
        "addr_type": e.addr_type,
        "company_id": e.company_id,
        "name": name,
        "obs_mono_ms": e.obs_mono_ms,
        "ble_cod": ble_cod,
    })
}

fn auth_to_u8(a: AuthMini) -> u8 {
    match a {
        AuthMini::Open => 0,
        AuthMini::Wep => 1,
        AuthMini::Wpa => 2,
        AuthMini::Wpa2 => 3,
        AuthMini::WpaWpa2 => 4,
        AuthMini::Wpa2Ent => 5,
        AuthMini::Wpa3 => 6,
        AuthMini::Wpa2Wpa3 => 7,
        AuthMini::Unknown => 255,
    }
}

fn ssid_bytes_to_wigle_string(ssid: &[u8], declared_len: usize) -> String {
    wardriver_core::wigle::ssid_bytes_to_wigle_string(ssid, declared_len)
}

fn parse_u8_channel_list(v: Option<&Value>, field: &str) -> Result<Vec<u8>, String> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        out.push(
            item.as_u64()
                .ok_or_else(|| format!("{field} entry must be u8"))? as u8,
        );
    }
    Ok(out)
}

fn parse_u32_list(v: Option<&Value>, field: &str) -> Result<Vec<u32>, String> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        out.push(
            item.as_u64()
                .ok_or_else(|| format!("{field} entry must be u32"))? as u32,
        );
    }
    Ok(out)
}

fn parse_channel_plan_sync(req: &Value) -> Result<ChannelPlanSyncV1, String> {
    let band_mode_str = req
        .get("band_mode")
        .and_then(Value::as_str)
        .unwrap_or("auto");
    let band_mode = if band_mode_str == "manual" {
        band_mode::MANUAL
    } else {
        band_mode::AUTO_BALANCE
    };
    let mut pins = Vec::new();
    if let Some(arr) = req.get("pins").and_then(Value::as_array) {
        for item in arr {
            let sniffer_id = item
                .get("sniffer_id")
                .and_then(Value::as_u64)
                .ok_or("pins.sniffer_id")? as u32;
            let channel = item.get("channel").and_then(Value::as_u64).unwrap_or(0) as u8;
            pins.push((sniffer_id, channel));
        }
    }
    Ok(ChannelPlanSyncV1 {
        schema_version: wardriver_packlink::CHANNEL_PLAN_SYNC_SCHEMA,
        revision: req.get("revision").and_then(Value::as_u64).unwrap_or(0) as u32,
        include_2_4_ghz: req
            .get("include_2_4_ghz")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        include_5_ghz: req
            .get("include_5_ghz")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        include_dfs: req
            .get("include_dfs")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        dwell_ms: req.get("dwell_ms").and_then(Value::as_u64).unwrap_or(200) as u16,
        band_mode,
        channels_2_4: parse_u8_channel_list(req.get("channels_2_4"), "channels_2_4")?,
        channels_5: parse_u8_channel_list(req.get("channels_5"), "channels_5")?,
        channels_5_dfs: parse_u8_channel_list(req.get("channels_5_dfs"), "channels_5_dfs")?,
        sniffers_2_4: parse_u32_list(req.get("sniffers_2_4"), "sniffers_2_4")?,
        sniffers_5: parse_u32_list(req.get("sniffers_5"), "sniffers_5")?,
        pins,
    })
}

fn parse_policy_sync(req: &Value) -> Result<PolicySyncV1, String> {
    let mut channels = [0u8; CHANNELS_MAX];
    if let Some(arr) = req.get("channels").and_then(Value::as_array) {
        for (i, item) in arr.iter().take(CHANNELS_MAX).enumerate() {
            channels[i] = item.as_u64().unwrap_or(0) as u8;
        }
    }
    let mut roster = Vec::new();
    if let Some(arr) = req.get("roster").and_then(Value::as_array) {
        for item in arr {
            let sniffer_id = item
                .get("sniffer_id")
                .and_then(Value::as_u64)
                .ok_or("roster.sniffer_id")? as u32;
            let mac = parse_bssid(item.get("mac"))?;
            let fleet_slot = item.get("fleet_slot").and_then(Value::as_u64).unwrap_or(0) as u8;
            let role = item.get("role").and_then(Value::as_u64).unwrap_or(0) as u8;
            roster.push(RosterEntryV1 {
                sniffer_id,
                mac,
                fleet_slot,
                role: role.min(1),
            });
        }
    }
    let mut denylist_bssids = Vec::new();
    if let Some(arr) = req.get("denylist_bssids").and_then(Value::as_array) {
        for item in arr {
            denylist_bssids.push(parse_bssid(Some(item))?);
        }
    }
    let mut denylist_ssids = Vec::new();
    if let Some(arr) = req.get("denylist_ssids").and_then(Value::as_array) {
        for item in arr {
            denylist_ssids.push(
                item.as_str()
                    .ok_or("denylist_ssids entry must be string")?
                    .to_string(),
            );
        }
    }
    Ok(PolicySyncV1 {
        scan_allowed: req
            .get("scan_allowed")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        wardrive_suppressed: req
            .get("wardrive_suppressed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        geofence_enabled: req
            .get("geofence_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        geofence_lat_min: req
            .get("geofence_lat_min")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        geofence_lat_max: req
            .get("geofence_lat_max")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        geofence_lon_min: req
            .get("geofence_lon_min")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        geofence_lon_max: req
            .get("geofence_lon_max")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        denylist_bssids,
        denylist_ssids,
        roster,
        dwell_ms: req.get("dwell_ms").and_then(Value::as_u64).unwrap_or(0) as u16,
        chan_count: req.get("chan_count").and_then(Value::as_u64).unwrap_or(0) as u8,
        channels,
        detection_disable_wifi_mask: req
            .get("detection_disable_wifi_mask")
            .or_else(|| req.get("flock_disable_wifi_mask"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        detection_disable_ble_mask: req
            .get("detection_disable_ble_mask")
            .or_else(|| req.get("flock_disable_ble_mask"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        ssid_watch: parse_string_list(req.get("ssid_watch"), "ssid_watch")?,
        remote_id_disable_mask: req
            .get("remote_id_disable_mask")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        ble_active_scan: req
            .get("ble_active_scan")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_string_list(v: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        out.push(
            item.as_str()
                .ok_or_else(|| format!("{field} entry must be string"))?
                .to_string(),
        );
    }
    Ok(out)
}

fn msg_type_name(t: MsgType) -> &'static str {
    match t {
        MsgType::GatewayInfo => "GatewayInfo",
        MsgType::PolicySync => "PolicySync",
        MsgType::ControlCmd => "ControlCmd",
        MsgType::EventNotify => "EventNotify",
        MsgType::ObservationBulk => "ObservationBulk",
        MsgType::SpoolReplay => "SpoolReplay",
        MsgType::TimeSync => "TimeSync",
        MsgType::SnifferStatus => "SnifferStatus",
        MsgType::CaptureCmd => "CaptureCmd",
        MsgType::CaptureStatus => "CaptureStatus",
        MsgType::PhoneHello => "PhoneHello",
        MsgType::TrustOk => "TrustOk",
        MsgType::TrustReject => "TrustReject",
        MsgType::SoftApConfig => "SoftApConfig",
        MsgType::TrustForget => "TrustForget",
        MsgType::PolicySyncAck => "PolicySyncAck",
        MsgType::ChannelPlanSync => "ChannelPlanSync",
        MsgType::ChannelPlanSyncAck => "ChannelPlanSyncAck",
        MsgType::FleetUnbindNotify => "FleetUnbindNotify",
        MsgType::ProbeObservationBulk => "ProbeObservationBulk",
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn bssid_to_colon_hex(bssid: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5]
    )
}

fn uas_id_to_detail_field(uas_id: &[u8; 20]) -> String {
    let end = uas_id.iter().position(|&b| b == 0).unwrap_or(uas_id.len());
    String::from_utf8_lossy(&uas_id[..end]).into_owned()
}

fn remote_id_e7_to_opt(e7: i32) -> Option<f64> {
    if e7 == 0 {
        None
    } else {
        Some(f64::from(e7) / 10_000_000.0)
    }
}

fn remote_id_dm_to_opt_m(dm: u16) -> Option<f32> {
    if dm == 0 {
        None
    } else {
        Some(dm as f32 / 10.0)
    }
}

fn remote_id_heading_opt(heading_deg: u16) -> Option<u16> {
    if heading_deg <= 359 {
        Some(heading_deg)
    } else {
        None
    }
}

fn remote_id_timestamp_sec_opt(timestamp_ds: u16) -> Option<f32> {
    if timestamp_ds == 0 || timestamp_ds == 0xFFFF {
        None
    } else {
        Some(timestamp_ds as f32 / 10.0)
    }
}

fn remote_id_alert_detail_json(alert: &wardriver_proto::RemoteIdAlertV1) -> String {
    let mut obj = json!({
        "mac": bssid_to_colon_hex(&alert.src),
        "channel": alert.channel,
        "rssi": alert.rssi,
        "method": alert.detection_method,
        "signal_kind": alert.signal_kind,
        "uas_id": uas_id_to_detail_field(&alert.uas_id),
    });
    if alert.id_type > 0 {
        obj["id_type"] = json!(alert.id_type);
    }
    if alert.ua_type > 0 {
        obj["ua_type"] = json!(alert.ua_type);
    }
    if alert.status > 0 {
        obj["status"] = json!(alert.status);
    }
    if let Some(h) = remote_id_heading_opt(alert.heading_deg) {
        obj["heading_deg"] = json!(h);
    }
    if let Some(speed) = remote_id_dm_to_opt_m(alert.speed_h_dmps) {
        obj["speed_h_mps"] = json!(speed);
    }
    if let Some(alt) = remote_id_dm_to_opt_m(alert.alt_geo_dm) {
        obj["alt_geo_m"] = json!(alt);
    }
    if let Some(hag) = remote_id_dm_to_opt_m(alert.height_agl_dm) {
        obj["height_agl_m"] = json!(hag);
    }
    if alert.height_type > 0 {
        obj["height_type"] = json!(alert.height_type);
    }
    if alert.horiz_accuracy > 0 {
        obj["horiz_accuracy"] = json!(alert.horiz_accuracy);
    }
    if alert.vert_accuracy > 0 {
        obj["vert_accuracy"] = json!(alert.vert_accuracy);
    }
    if alert.speed_accuracy > 0 {
        obj["speed_accuracy"] = json!(alert.speed_accuracy);
    }
    if alert.baro_accuracy > 0 {
        obj["baro_accuracy"] = json!(alert.baro_accuracy);
    }
    if alert.ts_accuracy > 0 {
        obj["ts_accuracy"] = json!(alert.ts_accuracy);
    }
    if let Some(ts) = remote_id_timestamp_sec_opt(alert.timestamp_ds) {
        obj["timestamp_sec"] = json!(ts);
    }
    let operator_id = uas_id_to_detail_field(&alert.operator_id);
    if !operator_id.is_empty() {
        obj["operator_id"] = json!(operator_id);
    }
    if alert.operator_id_type > 0 {
        obj["operator_id_type"] = json!(alert.operator_id_type);
    }
    if let Some(lat) = remote_id_e7_to_opt(alert.operator_lat_e7) {
        obj["operator_lat"] = json!(lat);
    }
    if let Some(lon) = remote_id_e7_to_opt(alert.operator_lon_e7) {
        obj["operator_lon"] = json!(lon);
    }
    if alert.operator_location_type > 0 {
        obj["operator_location_type"] = json!(alert.operator_location_type);
    }
    obj.to_string()
}

fn parse_bssid(v: Option<&Value>) -> Result<[u8; 6], String> {
    let Some(s) = v.and_then(Value::as_str) else {
        return Err("ap.bssid required (AA:BB:CC:DD:EE:FF)".into());
    };
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return Err(format!("bad bssid '{s}'"));
    }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        out[i] = u8::from_str_radix(p, 16).map_err(|_| format!("bad bssid byte '{p}'"))?;
    }
    Ok(out)
}

pub(crate) fn parse_ap_entry(v: Option<&Value>) -> Result<ApEntryV1, String> {
    let Some(ap) = v else {
        return Err("ap object required".into());
    };
    let bssid = parse_bssid(ap.get("bssid"))?;
    let channel = ap.get("channel").and_then(Value::as_u64).unwrap_or(0) as u8;
    let rssi = ap.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i8;
    let auth = ap.get("auth").and_then(Value::as_u64).unwrap_or(255) as u8;
    let auth = match auth {
        0 => AuthMini::Open,
        1 => AuthMini::Wep,
        2 => AuthMini::Wpa,
        3 => AuthMini::Wpa2,
        4 => AuthMini::WpaWpa2,
        5 => AuthMini::Wpa2Ent,
        6 => AuthMini::Wpa3,
        7 => AuthMini::Wpa2Wpa3,
        _ => AuthMini::Unknown,
    };
    let ssid_str = ap.get("ssid").and_then(Value::as_str).unwrap_or("");
    let mut ssid = [0u8; 32];
    let nbytes = ssid_str.len().min(32);
    ssid[..nbytes].copy_from_slice(&ssid_str.as_bytes()[..nbytes]);
    let obs_mono_ms = ap.get("obs_mono_ms").and_then(Value::as_u64).unwrap_or(0) as u32;
    Ok(ApEntryV1 {
        bssid,
        channel,
        rssi,
        auth,
        ssid_len: nbytes as u8,
        ssid,
        obs_mono_ms,
    })
}

pub(crate) fn parse_ble_entry(v: Option<&Value>) -> Result<BleEntryV1, String> {
    let Some(ble) = v else {
        return Err("ble object required".into());
    };
    let addr = parse_bssid(ble.get("addr").or_else(|| ble.get("bssid")))?;
    let rssi = ble.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i8;
    let addr_type = ble.get("addr_type").and_then(Value::as_u64).unwrap_or(0) as u8;
    let company_id = ble.get("company_id").and_then(Value::as_u64).unwrap_or(0) as u16;
    let name_str = ble.get("name").and_then(Value::as_str).unwrap_or("");
    let mut name = [0u8; 22];
    let nbytes = name_str.len().min(22);
    name[..nbytes].copy_from_slice(&name_str.as_bytes()[..nbytes]);
    let obs_mono_ms = ble.get("obs_mono_ms").and_then(Value::as_u64).unwrap_or(0) as u32;
    Ok(BleEntryV1 {
        addr,
        rssi,
        addr_type,
        company_id,
        name,
        obs_mono_ms,
    })
}

pub(crate) fn parse_ap_rcois(v: Option<&Value>) -> String {
    v.and_then(|ap| ap.get("rcois"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn parse_ble_extras(v: Option<&Value>) -> ScanEntryExtras {
    let mut extras = ScanEntryExtras::default();
    let Some(ble) = v else {
        return extras;
    };
    if let Some(arr) = ble.get("ble_cod").and_then(Value::as_array) {
        if arr.len() == 3 {
            if let (Some(a), Some(b), Some(c)) = (arr[0].as_u64(), arr[1].as_u64(), arr[2].as_u64())
            {
                extras.ble_cod = Some([a as u8, b as u8, c as u8]);
            }
        }
    }
    extras
}

pub(crate) fn apply_ephemeral_enrich_fields(
    state: &mut EnrichmentState,
    req: &Value,
) -> Result<(), String> {
    state.trusted_clock = req
        .get("trusted_clock")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    state.gps_usable = req
        .get("gps_usable")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    state.inside_home_geofence = req
        .get("inside_home_geofence")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    state.scan_held = req
        .get("scan_held")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    state.fast_mode = req
        .get("fast_mode")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // next_id and mac_table persist in the wardrive session — not reset per enrich call.

    state.gps_hist.clear();
    if let Some(arr) = req.get("gps_hist").and_then(Value::as_array) {
        for item in arr {
            let unix_ms = item
                .get("unix_ms")
                .and_then(Value::as_i64)
                .ok_or("gps_hist.unix_ms")?;
            let lat = item
                .get("lat")
                .and_then(Value::as_f64)
                .ok_or("gps_hist.lat")?;
            let lon = item
                .get("lon")
                .and_then(Value::as_f64)
                .ok_or("gps_hist.lon")?;
            state.gps_hist.push_back((
                unix_ms,
                GpsFix {
                    utc_unix_ms: unix_ms,
                    lat,
                    lon,
                    altitude_m: item
                        .get("altitude_m")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                    hdop: None,
                    accuracy_m: item
                        .get("accuracy_m")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                    fix_quality: 1,
                },
            ));
        }
    }

    state.denylist_bssids.clear();
    if let Some(arr) = req.get("denylist_bssids").and_then(Value::as_array) {
        for item in arr {
            state.denylist_bssids.push(parse_bssid(Some(item))?);
        }
    }
    state.denylist_ssids.clear();
    if let Some(arr) = req.get("denylist_ssids").and_then(Value::as_array) {
        for item in arr {
            let s = item
                .as_str()
                .ok_or("denylist_ssids entry must be string")?
                .to_string();
            state.denylist_ssids.push(s);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wardriver_packlink::{pack_frame, pack_observation_bulk, MsgType, ObservationBulkV1};
    use wardriver_proto::{pack_scan_frag, ApEntryV1, AuthMini, ScanFragV1Header};

    #[test]
    fn unpack_json_ok() {
        let wire = pack_frame(MsgType::GatewayInfo, 0, 7, b"ping");
        let out = unpack_packlink_frame_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["msg_type"], 1);
        assert_eq!(v["seq"], 7);
    }

    #[test]
    fn unpack_json_bad_crc() {
        let mut wire = pack_frame(MsgType::TimeSync, 0, 1, &[0u8; 4]);
        let last = wire.len() - 1;
        wire[last] ^= 0xFF;
        let out = unpack_packlink_frame_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn pack_policy_sync_json_ok() {
        let req = json!({
            "scan_allowed": true,
            "roster": [],
            "channels": [1, 6, 11],
            "chan_count": 3,
            "ble_active_scan": true,
        });
        let out = pack_policy_sync_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert!(v["wire_hex"].as_str().unwrap_or("").len() > 8);
        // Verify the wire round-trips ble_active_scan=true.
        let wire = hex_decode(v["wire_hex"].as_str().unwrap_or(""));
        let frame = unpack_frame(&wire).expect("frame");
        let policy = wardriver_packlink::unpack_policy_sync(frame.payload).expect("policy");
        assert!(policy.ble_active_scan);
    }

    #[test]
    fn format_wifi_csv_line_json_ok() {
        let req = json!({
            "observation": {
                "observed_unix_ms": 1_700_000_000_000i64,
                "bssid": "AA:BB:CC:DD:EE:01",
                "ssid": "Test",
                "auth_mode": "[WPA2-PSK-CCMP][ESS]",
                "channel": 6,
                "frequency_mhz": 2437,
                "rssi": -55,
                "lat": 37.77,
                "lon": -122.42,
            }
        });
        let out = format_wifi_csv_line_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert!(v["line"]
            .as_str()
            .unwrap_or("")
            .contains("aa:bb:cc:dd:ee:01"));
    }

    #[test]
    fn unpack_sniffer_status_json_ok() {
        let row = wardriver_packlink::SnifferStatusV1 {
            sniffer_id: 5,
            mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
            last_seen_ms: 999,
            state: 1,
            role: 0,
            cpu_load_pct: 33,
            heap_avail_pct: 77,
            fleet_slot: 2,
            reboot_count: 0,
            adoption_state: 1,
            plan_ext: None,
            adopt_ext: None,
        };
        let wire = wardriver_packlink::pack_sniffer_status(&row, 12_345, 1);
        let out = unpack_sniffer_status_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["count"], 1);
        assert_eq!(v["gateway_mono_ms"], 12_345);
        assert_eq!(v["rows"][0]["sniffer_id"], 5);
        assert_eq!(v["rows"][0]["adoption_state"], 1);
    }

    #[test]
    fn unpack_policy_sync_ack_json_ok() {
        let ack = wardriver_packlink::PolicySyncAckV1 {
            policy_seq: 7,
            result: wardriver_packlink::policy_sync_ack_result::OK,
            error_code: wardriver_packlink::policy_sync_ack_error::NONE,
        };
        let wire = wardriver_packlink::pack_policy_sync_ack(&ack, 3);
        let out = unpack_policy_sync_ack_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["policy_seq"], 7);
        assert_eq!(v["result"], 0);
        assert_eq!(v["error_code"], 0);
    }

    #[test]
    fn enrich_ble_ap_json_stores_observation() {
        let _ = enrichment_session_close_json(r#"{"session_id":1}"#);
        let open = enrichment_session_open_json(r#"{"session_id":1}"#);
        let open_v: Value = serde_json::from_str(&open).expect("json");
        assert_eq!(open_v["ok"], true);
        let req = json!({
            "session_id": 1,
            "hdr_scan_end_ms": 30_000,
            "hdr_scan_end_unix_ms": 1_777_188_600_000u64,
            "sniffer_id": 5,
            "trusted_clock": true,
            "gps_usable": true,
            "gps_hist": [{
                "unix_ms": 1_777_188_600_000i64,
                "lat": 37.0,
                "lon": -122.0,
                "accuracy_m": 5.0,
                "altitude_m": 10.0,
            }],
            "ble": {
                "addr": "11:22:33:44:55:66",
                "rssi": -70,
                "addr_type": 1,
                "company_id": 76,
                "name": "Pixel Buds",
                "obs_mono_ms": 30_000,
            }
        });
        let out = enrich_ble_ap_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["outcome"], "stored");
        assert_eq!(v["observation"]["signal_kind"], "BLE");
        assert_eq!(v["observation"]["auth_mode"], "76");
    }

    #[test]
    fn format_ble_csv_line_json_ok() {
        let req = json!({
            "observation": {
                "observed_unix_ms": 1_777_188_600_000i64,
                "bssid": "11:22:33:44:55:66",
                "ssid": "Pixel Buds",
                "auth_mode": "76",
                "channel": 0,
                "rssi": -70,
                "lat": 37.0,
                "lon": -122.0,
                "altitude_m": 10,
                "accuracy_m": 5.0,
            }
        });
        let out = format_ble_csv_line_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        let line = v["line"].as_str().unwrap_or("");
        assert!(line.contains("11:22:33:44:55:66"));
        assert!(line.ends_with(",BLE"));
        assert!(line.contains(",7936,"));
        assert!(!line.contains("Unknown [LE]"));
    }

    #[test]
    fn unpack_observation_bulk_json_with_ble_scan_frag() {
        let mut name = [0u8; 22];
        name[..10].copy_from_slice(b"Pixel Buds");
        let entry = wardriver_proto::BleEntryV1 {
            addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            rssi: -70,
            addr_type: 1,
            company_id: 0x004C,
            name,
            obs_mono_ms: 30_000,
        };
        let hdr = wardriver_proto::BleScanFragV1Header {
            flags: wardriver_proto::FRAG_FLAG_TRANSPORT_END,
            sniffer_id: 5,
            seq: 7,
            scan_batch_id: 9,
            sniffer_boot_id: 0x1122_3344,
            sniffer_scan_end_ms: 30_000,
            sniffer_scan_end_unix_ms: 1_777_188_600_000,
            frag_idx: 0,
            frag_total: 1,
            entry_count: 1,
        };
        let scan_wire =
            wardriver_proto::pack_ble_scan_frag(hdr, std::slice::from_ref(&entry)).expect("ble");
        let bulk = ObservationBulkV1 {
            batch_id: 9,
            sniffer_id: 5,
            scan_end_unix_ms: 1_777_188_600_000,
            wifi_ap_count: 0,
            ble_count: 1,
            payload: scan_wire,
        };
        let frame = pack_observation_bulk(&bulk, 7, 0);
        let out = unpack_observation_bulk_json(&frame);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["aps"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(v["bles"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(v["bles"][0]["name"], "Pixel Buds");
    }

    #[test]
    fn unpack_observation_bulk_json_with_scan_frag() {
        let mut ssid = [0u8; 32];
        ssid[..4].copy_from_slice(b"Test");
        let ap = ApEntryV1 {
            bssid: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01],
            channel: 6,
            rssi: -55,
            auth: AuthMini::Wpa2,
            ssid_len: 4,
            ssid,
            obs_mono_ms: 50_000,
        };
        let hdr = ScanFragV1Header {
            flags: 0,
            sniffer_id: 3,
            seq: 1,
            scan_batch_id: 99,
            sniffer_boot_id: 1,
            sniffer_scan_end_ms: 50_000,
            sniffer_scan_end_unix_ms: 1_700_000_000_000,
            frag_idx: 0,
            frag_total: 1,
            ap_count: 1,
        };
        let scan_wire = pack_scan_frag(hdr, &[ap]).expect("scan frag");
        let bulk = ObservationBulkV1 {
            batch_id: 99,
            sniffer_id: 3,
            scan_end_unix_ms: 1_700_000_000_000,
            wifi_ap_count: 1,
            ble_count: 0,
            payload: scan_wire,
        };
        let frame = pack_observation_bulk(&bulk, 7, 0);
        let out = unpack_observation_bulk_json(&frame);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["aps"].as_array().map(|a| a.len()), Some(1));
    }

    #[test]
    fn unpack_sniffer_status_rejects_event_notify() {
        let ev = wardriver_packlink::EventNotifyV1 {
            event_kind: 1,
            sniffer_id: 3,
            unix_ms: 1_700_000_000_000,
            payload_len: 0,
            payload: Vec::new(),
        };
        let wire = wardriver_packlink::pack_event_notify(&ev, 1);
        let out = unpack_sniffer_status_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn pack_time_sync_json_roundtrip() {
        let req = json!({
            "phone_unix_ms": 1_700_000_000_000u64,
            "phone_mono_ms": 12345,
            "gateway_mono_ms": 0,
            "time_set_seq": 7,
            "seq": 9,
        });
        let packed = pack_time_sync_json(&req.to_string());
        let v: Value = serde_json::from_str(&packed).expect("json");
        assert_eq!(v["ok"], true);
        let wire_hex = v["wire_hex"].as_str().expect("wire_hex");
        let wire = hex_decode(wire_hex);
        let out = unpack_time_sync_json(&wire);
        let back: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(back["ok"], true);
        assert_eq!(back["time_set_seq"], 7);
    }

    #[test]
    fn unpack_spool_replay_json_ok() {
        let chunk = wardriver_packlink::SpoolReplayV1 {
            spool_offset: 0,
            chunk_total: 2,
            chunk_index: 0,
            payload: b"PLNK".to_vec(),
        };
        let wire = wardriver_packlink::pack_spool_replay(&chunk, 1);
        let out = unpack_spool_replay_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["chunk_index"], 0);
    }

    #[test]
    fn extract_packlink_frames_from_spool_buffer() {
        let inner = pack_frame(MsgType::GatewayInfo, 0, 3, b"info");
        let chunk = wardriver_packlink::SpoolReplayV1 {
            spool_offset: 0,
            chunk_total: 1,
            chunk_index: 0,
            payload: inner.clone(),
        };
        let (frames, remainder) = extract_packlink_frames(&inner);
        assert_eq!(frames.len(), 1);
        assert!(remainder.is_empty());
        let _ = chunk;
    }

    #[test]
    fn unpack_event_notify_lpr_camera_wifi_json() {
        let alert = wardriver_proto::DetectionAlertV1 {
            sniffer_id: 9,
            seq: 1,
            sniffer_boot_id: 2,
            sniffer_scan_end_ms: 50_000,
            sniffer_scan_end_unix_ms: 1_700_000_000_000,
            src: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            detection_method: 1,
            channel: 6,
            rssi_min: -70,
            rssi_max: -50,
            rssi_avg_q8: -60 * 256,
            wildcard_count: 0,
            first_seen_ms32: 1,
            last_seen_ms32: 2,
            primary_sig_matched: 1,
        };
        let wolf = wardriver_proto::pack_detection_alert(alert);
        let ev = wardriver_packlink::EventNotifyV1 {
            event_kind: wardriver_packlink::event_kind::DETECTION_WIFI,
            sniffer_id: 9,
            unix_ms: 1_700_000_000_100,
            payload_len: wolf.len() as u16,
            payload: wolf,
        };
        let wire = wardriver_packlink::pack_event_notify(&ev, 4);
        let out = unpack_event_notify_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["kind"], "DETECTION_WIFI");
        assert_eq!(v["sniffer_id"], 9);
        assert_eq!(v["detection_method_id"], 1);
        assert!(v["detail"].as_str().unwrap().contains("method=1"));
    }

    #[test]
    fn unpack_event_notify_remote_id_json_detail() {
        let alert = wardriver_proto::RemoteIdAlertV1 {
            sniffer_id: 9,
            seq: 1,
            sniffer_boot_id: 2,
            sniffer_scan_end_ms: 50_000,
            sniffer_scan_end_unix_ms: 1_700_000_000_000,
            src: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            channel: 6,
            rssi: -55,
            detection_method: 64,
            signal_kind: 0,
            uas_id: {
                let mut id = [0u8; 20];
                id[..8].copy_from_slice(b"FA3ABC12");
                id
            },
            drone_lat_e7: 386_270_890,
            drone_lon_e7: -902_004_040,
            alt_geo_dm: 1200,
            heading_deg: 90,
            speed_h_dmps: 52,
            operator_lat_e7: 386_260_000,
            operator_lon_e7: -901_990_000,
            flags: 0,
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
            operator_id: {
                let mut id = [0u8; 20];
                id[..6].copy_from_slice(b"PILOT1");
                id
            },
        };
        let wolf = wardriver_proto::pack_remote_id_alert(alert);
        let ev = wardriver_packlink::EventNotifyV1 {
            event_kind: wardriver_packlink::event_kind::REMOTE_ID,
            sniffer_id: 9,
            unix_ms: 1_700_000_000_100,
            payload_len: wolf.len() as u16,
            payload: wolf,
        };
        let wire = wardriver_packlink::pack_event_notify(&ev, 5);
        let out = unpack_event_notify_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["kind"], "REMOTE_ID");
        assert_eq!(v["detection_method_id"], 64);
        let detail: Value =
            serde_json::from_str(v["detail"].as_str().unwrap()).expect("detail json");
        assert_eq!(detail["mac"], "AA:BB:CC:DD:EE:FF");
        assert_eq!(detail["method"], 64);
        assert_eq!(detail["uas_id"], "FA3ABC12");
        assert_eq!(detail["id_type"], 1);
        assert_eq!(detail["operator_id"], "PILOT1");
        assert!(detail["operator_lat"].is_number());
    }

    #[test]
    fn pack_channel_plan_sync_json_ok() {
        let req = json!({
            "revision": 3,
            "include_2_4_ghz": true,
            "include_5_ghz": true,
            "include_dfs": false,
            "dwell_ms": 200,
            "band_mode": "auto",
            "channels_2_4": [1, 6, 11],
            "channels_5": [36, 40],
            "channels_5_dfs": [],
            "sniffers_2_4": [],
            "sniffers_5": [],
            "pins": [],
            "seq": 9,
        });
        let out = pack_channel_plan_sync_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        let wire = hex_decode(v["wire_hex"].as_str().unwrap());
        let frame = wardriver_packlink::unpack_frame(&wire).expect("frame");
        assert_eq!(frame.msg_type, MsgType::ChannelPlanSync);
        let plan = wardriver_packlink::unpack_channel_plan_sync(frame.payload).expect("plan");
        assert_eq!(plan.revision, 3);
        assert_eq!(plan.dwell_ms, 200);
        assert_eq!(plan.channels_2_4, vec![1, 6, 11]);
    }

    #[test]
    fn unpack_channel_plan_sync_ack_json_ok() {
        let ack = wardriver_packlink::ChannelPlanSyncAckV1 {
            plan_seq: 9,
            revision: 3,
            result: wardriver_packlink::channel_plan_sync_ack_result::OK,
            error_code: wardriver_packlink::channel_plan_sync_ack_error::NONE,
            hop_cycle_ms: 1200,
            unassigned_count: 0,
        };
        let wire = wardriver_packlink::pack_channel_plan_sync_ack(&ack, 4);
        let out = unpack_channel_plan_sync_ack_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["plan_seq"], 9);
        assert_eq!(v["revision"], 3);
        assert_eq!(v["hop_cycle_ms"], 1200);
    }

    #[test]
    fn unpack_sniffer_status_json_plan_ext() {
        let row = wardriver_packlink::SnifferStatusV1 {
            sniffer_id: 5,
            mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
            last_seen_ms: 999,
            state: 1,
            role: 0,
            cpu_load_pct: 33,
            heap_avail_pct: 77,
            fleet_slot: 2,
            reboot_count: 0,
            adoption_state: 1,
            plan_ext: Some(wardriver_packlink::SnifferStatusPlanExtV1 {
                five_ghz_capable: 1,
                assigned_band: 2,
                pinned_channel: 36,
                assigned_chan_count: 3,
                assigned_channels_preview: [36, 40, 44, 0],
            }),
            adopt_ext: None,
        };
        let wire = wardriver_packlink::pack_sniffer_status(&row, 12_345, 1);
        let out = unpack_sniffer_status_json(&wire);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["rows"][0]["five_ghz_capable"], 1);
        assert_eq!(v["rows"][0]["assigned_band"], 2);
        assert_eq!(v["rows"][0]["pinned_channel"], 36);
        assert_eq!(v["rows"][0]["assigned_chan_count"], 3);
    }

    #[test]
    fn pack_policy_sync_json_high_bit_sniffer_ids() {
        let req = json!({
            "scan_allowed": true,
            "roster": [
                {
                    "sniffer_id": 5,
                    "mac": "02:11:22:33:44:55",
                    "fleet_slot": 1,
                    "role": 0
                },
                {
                    "sniffer_id": 0x8000_0000u64,
                    "mac": "02:AA:BB:CC:DD:01",
                    "fleet_slot": 2,
                    "role": 0
                },
                {
                    "sniffer_id": 0xDEAD_BEEFu64,
                    "mac": "02:AA:BB:CC:DD:02",
                    "fleet_slot": 3,
                    "role": 0
                },
                {
                    "sniffer_id": 0xFFFF_FFFFu64,
                    "mac": "02:AA:BB:CC:DD:03",
                    "fleet_slot": 4,
                    "role": 0
                },
            ],
            "channels": [1, 6, 11],
            "chan_count": 3,
        });
        let out = pack_policy_sync_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        let wire = hex_decode(v["wire_hex"].as_str().unwrap());
        let frame = wardriver_packlink::unpack_frame(&wire).expect("frame");
        let policy = wardriver_packlink::unpack_policy_sync(frame.payload).expect("policy");
        assert_eq!(policy.roster.len(), 4);
        assert_eq!(policy.roster[0].sniffer_id, 5);
        assert_eq!(policy.roster[1].sniffer_id, 0x8000_0000);
        assert_eq!(policy.roster[2].sniffer_id, 0xDEAD_BEEF);
        assert_eq!(policy.roster[3].sniffer_id, 0xFFFF_FFFF);
    }

    #[test]
    fn pack_policy_sync_negative_sniffer_id_fails() {
        let req = json!({
            "scan_allowed": true,
            "roster": [{
                "sniffer_id": -1,
                "mac": "02:11:22:33:44:55",
                "fleet_slot": 1,
                "role": 0
            }],
        });
        let out = pack_policy_sync_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn pack_policy_sync_roster_role_json() {
        let req = json!({
            "scan_allowed": true,
            "roster": [{
                "sniffer_id": 5,
                "mac": "02:11:22:33:44:55",
                "fleet_slot": 1,
                "role": 1
            }],
        });
        let out = pack_policy_sync_json(&req.to_string());
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["ok"], true);
        let wire = hex_decode(v["wire_hex"].as_str().unwrap());
        let frame = wardriver_packlink::unpack_frame(&wire).expect("frame");
        let policy = wardriver_packlink::unpack_policy_sync(frame.payload).expect("policy");
        assert_eq!(policy.roster.len(), 1);
        assert_eq!(policy.roster[0].role, 1);
    }

    fn hex_decode(hex: &str) -> Vec<u8> {
        let clean = hex.trim();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).expect("hex"))
            .collect()
    }
}
