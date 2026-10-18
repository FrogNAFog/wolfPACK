//! Wardrive-scoped [`EnrichmentState`] — per-run MacTable across JNI enrich calls.
//!
//! Control firmware keeps one MacTable in RAM for the whole session; mobile mirrors that
//! with an empty table at session open (not rehydrated from Room `network_summary`).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use crate::enrichment::EnrichmentState;

/// Process-global session table (one active wardrive session per phone at a time).
static SESSIONS: Mutex<Option<HashMap<i64, EnrichmentState>>> = Mutex::new(None);

fn sessions_mut() -> std::sync::MutexGuard<'static, Option<HashMap<i64, EnrichmentState>>> {
    let mut guard = SESSIONS.lock().expect("enrichment session mutex");
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

/// Open (or replace) an enrichment session keyed by Room `session._id`.
pub fn enrichment_session_open(session_id: i64) -> Result<(), String> {
    let mut guard = sessions_mut();
    let map = guard.as_mut().expect("session map");
    map.insert(session_id, EnrichmentState::default());
    Ok(())
}

/// Seed MacTable revisit positions from a JSON array (tests / optional callers).
pub fn enrichment_session_hydrate_mac_last(
    session_id: i64,
    mac_last: &Value,
) -> Result<(), String> {
    let Some(arr) = mac_last.as_array() else {
        return Ok(());
    };
    let mut guard = sessions_mut();
    let map = guard.as_mut().expect("session map");
    let state = map
        .get_mut(&session_id)
        .ok_or_else(|| format!("enrichment session {session_id} not open"))?;
    seed_mac_last_array(state, arr)
}

/// Seed CellTable revisit positions from a JSON array (tests / optional callers).
pub fn enrichment_session_hydrate_cell_last(
    session_id: i64,
    cell_last: &Value,
) -> Result<(), String> {
    let Some(arr) = cell_last.as_array() else {
        return Ok(());
    };
    let mut guard = sessions_mut();
    let map = guard.as_mut().expect("session map");
    let state = map
        .get_mut(&session_id)
        .ok_or_else(|| format!("enrichment session {session_id} not open"))?;
    for item in arr {
        let key = item
            .get("cell_key")
            .or_else(|| item.get("bssid"))
            .and_then(Value::as_str)
            .ok_or("cell_last.cell_key")?;
        if key.contains(':') {
            continue;
        }
        let lat = item
            .get("lat")
            .and_then(Value::as_f64)
            .ok_or("cell_last.lat")?;
        let lon = item
            .get("lon")
            .and_then(Value::as_f64)
            .ok_or("cell_last.lon")?;
        state.cell_table.seed(key, lat, lon);
    }
    Ok(())
}

/// Drop session state; idempotent when already closed.
pub fn enrichment_session_close(session_id: i64) {
    let mut guard = sessions_mut();
    if let Some(map) = guard.as_mut() {
        map.remove(&session_id);
    }
}

/// Run a closure against the live session state (MacTable + next_id persist across calls).
pub fn with_session_mut<R>(
    session_id: i64,
    f: impl FnOnce(&mut EnrichmentState) -> Result<R, String>,
) -> Result<R, String> {
    let mut guard = sessions_mut();
    let map = guard.as_mut().expect("session map");
    let state = map
        .get_mut(&session_id)
        .ok_or_else(|| format!("enrichment session {session_id} not open"))?;
    f(state)
}

/// Seed WiFi and BLE revisit lanes from a `mac_last` JSON array.
fn seed_mac_last_array(state: &mut EnrichmentState, arr: &[Value]) -> Result<(), String> {
    for item in arr {
        let bssid = parse_bssid_mac(item.get("bssid"))?;
        let lat = item
            .get("lat")
            .and_then(Value::as_f64)
            .ok_or("mac_last.lat")?;
        let lon = item
            .get("lon")
            .and_then(Value::as_f64)
            .ok_or("mac_last.lon")?;
        // Both lanes: hydrate revisit anchors without incrementing row cap.
        state.mac_table.seed_wigle_lanes(&bssid, lat, lon);
    }
    Ok(())
}

fn parse_bssid_mac(v: Option<&Value>) -> Result<[u8; 6], String> {
    let Some(s) = v.and_then(Value::as_str) else {
        return Err("mac_last.bssid required (AA:BB:CC:DD:EE:FF)".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_open_close_idempotent() {
        enrichment_session_close(99);
        enrichment_session_open(99).expect("open");
        enrichment_session_close(99);
        enrichment_session_close(99);
    }

    #[test]
    fn enrich_requires_open_session() {
        enrichment_session_close(1);
        let err = with_session_mut(1, |_| Ok(())).unwrap_err();
        assert!(err.contains("not open"));
    }
}
