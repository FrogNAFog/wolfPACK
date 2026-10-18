//! CSV enqueue gates + MacTable dedup — mirrors control ingest semantics.

use indexmap::IndexMap;
use std::collections::VecDeque;

use crate::cell_table::CellTable;
use crate::gps_anchor_mobile;
use wardriver_core::gps_anchor::{self, GpsFix, MAX_FIX_TIMELINE_DELTA_MS};
use wardriver_core::ingest_limits::phone_control_mac_table_limits;
use wardriver_core::mac_table::{
    consider_wigle_ble, consider_wigle_wifi, LaneFlags, MacEntry, MacTable,
};
use wardriver_core::wigle::{authmini_to_caps, ssid_bytes_to_wigle_string};
use wardriver_proto::{ApEntryV1, BleEntryV1, ScanEntryExtras};

use crate::observation::{CellObservation, Observation, ProbeObservation, SignalKind};

/// Same revisit radius as control [`REVISIT_RADIUS_M`].
pub const REVISIT_RADIUS_M: f64 = 50.0;

/// Separate dedup table for probe observations, keyed on (client MAC, SSID).
#[derive(Clone, Debug)]
pub struct ProbeMacTable {
    entries: IndexMap<([u8; 6], String), MacEntry>,
    max_entries: usize,
}

impl ProbeMacTable {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: IndexMap::with_capacity(max_entries),
            max_entries,
        }
    }

    pub fn allow(
        &mut self,
        key: &([u8; 6], String),
        lat: f64,
        lon: f64,
        radius_m: f64,
        rssi: Option<i8>,
    ) -> wardriver_core::mac_table::WigleRowDecision {
        let entry = self.entries.get(key);
        consider_wigle_wifi(entry, lat, lon, radius_m, rssi.unwrap_or(-100))
    }

    pub fn update_on_log(&mut self, key: &([u8; 6], String), lat: f64, lon: f64, rssi: i8) {
        let entry = self
            .entries
            .entry(key.clone())
            .or_insert_with(|| MacEntry::default());
        entry.last_lat = lat;
        entry.last_lon = lon;
        entry.wigle_row_count = entry.wigle_row_count.saturating_add(1);

        // Evict oldest (front = earliest insertion) when over capacity
        while self.entries.len() > self.max_entries {
            self.entries.shift_remove_index(0);
        }
    }
}

/// Enrichment context for a wardrive session.
#[derive(Debug)]
pub struct EnrichmentState {
    pub mac_table: MacTable,
    pub cell_table: CellTable,
    pub probe_mac_table: ProbeMacTable,
    pub gps_hist: VecDeque<(i64, GpsFix)>,
    pub trusted_clock: bool,
    pub gps_usable: bool,
    pub inside_home_geofence: bool,
    pub scan_held: bool,
    pub denylist_bssids: Vec<[u8; 6]>,
    pub denylist_ssids: Vec<String>,
    pub fast_mode: bool,
    pub next_id: i64,
}

impl Default for EnrichmentState {
    fn default() -> Self {
        let (mac_max, geo_max) = phone_control_mac_table_limits();
        Self {
            mac_table: MacTable::new(mac_max, geo_max),
            cell_table: CellTable::new(2048),
            probe_mac_table: ProbeMacTable::new(2048),
            gps_hist: VecDeque::new(),
            trusted_clock: true,
            gps_usable: true,
            inside_home_geofence: false,
            scan_held: false,
            denylist_bssids: Vec::new(),
            denylist_ssids: Vec::new(),
            fast_mode: false,
            next_id: 1,
        }
    }
}

/// Result of attempting to store one WiFi AP observation.
#[derive(Clone, Debug, PartialEq)]
pub enum EnrichOutcome {
    Stored(Observation),
    StoredCell(CellObservation),
    StoredProbe(ProbeObservation),
    Dropped(&'static str),
}

/// Why CSV enqueue is blocked, or None when allowed.
#[must_use]
pub fn csv_enqueue_block_reason(state: &EnrichmentState) -> Option<&'static str> {
    if !state.trusted_clock {
        return Some("csv_blocked_untrusted_clock");
    }
    if state.scan_held {
        return Some("csv_blocked_scan_held");
    }
    if state.inside_home_geofence {
        return Some("csv_blocked_home_zone");
    }
    if !state.gps_usable {
        return Some("csv_blocked_gps_unusable");
    }
    None
}

/// Mirrors control `csv_enqueue_allowed` gates (simplified host-testable subset).
#[must_use]
pub fn csv_enqueue_allowed(state: &EnrichmentState) -> bool {
    csv_enqueue_block_reason(state).is_none()
}

fn bssid_denied(bssid: &[u8; 6], deny: &[[u8; 6]]) -> bool {
    deny.iter().any(|d| d == bssid)
}

fn ssid_denied(ssid: &str, deny: &[String]) -> bool {
    deny.iter().any(|d| d == ssid)
}

/// Enrich one WiFi AP, storing formatted WiGLE `RCOIs` from the TLV sidecar.
pub fn enrich_wifi_ap_with_rcois(
    state: &mut EnrichmentState,
    session_id: i64,
    hdr_scan_end_ms: u32,
    hdr_scan_end_unix_ms: u64,
    sniffer_id: u32,
    ap: &ApEntryV1,
    rcois: &str,
) -> EnrichOutcome {
    if let Some(reason) = csv_enqueue_block_reason(state) {
        return EnrichOutcome::Dropped(reason);
    }
    if bssid_denied(&ap.bssid, &state.denylist_bssids) {
        return EnrichOutcome::Dropped("bssid_denylist");
    }
    if ssid_denied(
        &ssid_bytes_to_wigle_string(&ap.ssid, ap.ssid_len as usize),
        &state.denylist_ssids,
    ) {
        return EnrichOutcome::Dropped("ssid_denylist");
    }
    if ap.channel == 0 || wardriver_core::wigle::channel_to_frequency_mhz(ap.channel) == 0 {
        return EnrichOutcome::Dropped("invalid_channel");
    }
    let Some(obs_unix) = gps_anchor::observed_unix_ms_from_anchor(
        hdr_scan_end_ms,
        hdr_scan_end_unix_ms,
        ap.obs_mono_ms,
    ) else {
        return EnrichOutcome::Dropped("no_time_anchor");
    };
    let Some(fix) = gps_anchor_mobile::match_mobile_gps_fix_for_obs(
        &state.gps_hist,
        obs_unix,
        MAX_FIX_TIMELINE_DELTA_MS,
    ) else {
        return EnrichOutcome::Dropped("no_gps_match");
    };
    if state.fast_mode {
        let prev = state.mac_table.get(&ap.bssid);
        let peek = consider_wigle_wifi(prev, fix.lat, fix.lon, REVISIT_RADIUS_M, ap.rssi);
        if !peek.allow_row {
            return EnrichOutcome::Dropped("fast_mode_dedup");
        }
    }
    let allow = state.mac_table.allow(
        &ap.bssid,
        fix.lat,
        fix.lon,
        LaneFlags::WIGLE_WIFI,
        REVISIT_RADIUS_M,
        Some(ap.rssi),
    );
    if !allow.decision.allow_row {
        return EnrichOutcome::Dropped("mac_table_dedup");
    }
    let id = state.next_id;
    state.next_id += 1;
    let ssid = ssid_bytes_to_wigle_string(&ap.ssid, ap.ssid_len as usize);
    let auth_mode = authmini_to_caps(ap.auth);
    EnrichOutcome::Stored(Observation {
        id,
        session_id,
        sniffer_id,
        observed_unix_ms: obs_unix,
        lat: fix.lat,
        lon: fix.lon,
        altitude_m: fix.altitude_m.map(|a| a.round() as i32),
        accuracy_m: fix.accuracy_m,
        signal_kind: SignalKind::Wifi,
        bssid: ap.bssid,
        ssid,
        channel: ap.channel,
        frequency_mhz: wardriver_core::wigle::channel_to_frequency_mhz(ap.channel) as u32,
        rssi: ap.rssi,
        auth_mode,
        rcois: rcois.to_string(),
        ble_capabilities: String::new(),
        uploaded: false,
    })
}

/// BLE device display name from a compact `BleEntryV1` name field.
fn ble_name_from_entry(e: &BleEntryV1) -> String {
    let end = e.name.iter().position(|&x| x == 0).unwrap_or(e.name.len());
    String::from_utf8_lossy(&e.name[..end]).trim().to_string()
}

/// WiGLE `MfgrId` for BLE rows: decimal Bluetooth company ID when present.
fn ble_mfgr_id_string(company_id: u16) -> String {
    if company_id != 0 {
        company_id.to_string()
    } else {
        String::new()
    }
}

/// Enrich one BLE device using Class of Device from the TLV sidecar.
pub fn enrich_ble_entry_with_extras(
    state: &mut EnrichmentState,
    session_id: i64,
    hdr_scan_end_ms: u32,
    hdr_scan_end_unix_ms: u64,
    sniffer_id: u32,
    entry: &BleEntryV1,
    extras: &ScanEntryExtras,
) -> EnrichOutcome {
    if let Some(reason) = csv_enqueue_block_reason(state) {
        return EnrichOutcome::Dropped(reason);
    }
    if bssid_denied(&entry.addr, &state.denylist_bssids) {
        return EnrichOutcome::Dropped("bssid_denylist");
    }
    let Some(obs_unix) = gps_anchor::observed_unix_ms_from_anchor(
        hdr_scan_end_ms,
        hdr_scan_end_unix_ms,
        entry.obs_mono_ms,
    ) else {
        return EnrichOutcome::Dropped("no_time_anchor");
    };
    let Some(fix) = gps_anchor_mobile::match_mobile_gps_fix_for_obs(
        &state.gps_hist,
        obs_unix,
        MAX_FIX_TIMELINE_DELTA_MS,
    ) else {
        return EnrichOutcome::Dropped("no_gps_match");
    };
    if state.fast_mode {
        let prev = state.mac_table.get(&entry.addr);
        let peek = consider_wigle_ble(prev, fix.lat, fix.lon, REVISIT_RADIUS_M, entry.rssi);
        if !peek.allow_row {
            return EnrichOutcome::Dropped("fast_mode_dedup");
        }
    }
    let allow = state.mac_table.allow(
        &entry.addr,
        fix.lat,
        fix.lon,
        LaneFlags::WIGLE_BLE,
        REVISIT_RADIUS_M,
        Some(entry.rssi),
    );
    if !allow.decision.allow_row {
        return EnrichOutcome::Dropped("mac_table_dedup");
    }
    let id = state.next_id;
    state.next_id += 1;
    let name = ble_name_from_entry(entry);
    // `auth_mode` stores WiGLE BLE `MfgrId` (decimal company ID); not WiFi caps.
    let auth_mode = ble_mfgr_id_string(entry.company_id);
    let (ble_capabilities, frequency_code) = match extras.ble_cod {
        Some(cod) => wardriver_core::wigle_ble::ble_csv_fields_from_cod(cod[0], cod[1], cod[2]),
        None => wardriver_core::wigle_ble::ble_csv_fields_no_cod(),
    };
    EnrichOutcome::Stored(Observation {
        id,
        session_id,
        sniffer_id,
        observed_unix_ms: obs_unix,
        lat: fix.lat,
        lon: fix.lon,
        altitude_m: fix.altitude_m.map(|a| a.round() as i32),
        accuracy_m: fix.accuracy_m,
        signal_kind: SignalKind::Ble,
        bssid: entry.addr,
        ssid: name,
        channel: 0,
        frequency_mhz: u32::from(frequency_code),
        rssi: entry.rssi,
        auth_mode,
        rcois: String::new(),
        ble_capabilities,
        uploaded: false,
    })
}

/// Phone-sourced cell tower input (WiGLE cell row fields).
#[derive(Clone, Debug, PartialEq)]
pub struct CellTowerInput {
    pub cell_key: String,
    pub network_name: String,
    pub capabilities: String,
    pub cell_type: &'static str,
    pub frequency: u32,
    pub rssi: i8,
    pub observed_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
}

/// Enrich one phone-sourced cell tower observation.
pub fn enrich_cell(
    state: &mut EnrichmentState,
    session_id: i64,
    cell: &CellTowerInput,
) -> EnrichOutcome {
    if let Some(reason) = csv_enqueue_block_reason(state) {
        return EnrichOutcome::Dropped(reason);
    }
    if cell.cell_key.is_empty() {
        return EnrichOutcome::Dropped("empty_cell_key");
    }
    if state.fast_mode {
        let prev = state.cell_table.get(&cell.cell_key);
        let peek = prev.map_or(true, |e| {
            wardriver_core::mac_table::moved_at_least_m(
                e.last_lat,
                e.last_lon,
                cell.lat,
                cell.lon,
                REVISIT_RADIUS_M,
            )
        });
        if !peek {
            return EnrichOutcome::Dropped("fast_mode_dedup");
        }
    }
    let decision = state
        .cell_table
        .allow(&cell.cell_key, cell.lat, cell.lon, REVISIT_RADIUS_M);
    if !decision.allow_row {
        return EnrichOutcome::Dropped("cell_table_dedup");
    }
    let id = state.next_id;
    state.next_id += 1;
    EnrichOutcome::StoredCell(CellObservation {
        id,
        session_id,
        observed_unix_ms: cell.observed_unix_ms,
        lat: cell.lat,
        lon: cell.lon,
        altitude_m: cell.altitude_m,
        accuracy_m: cell.accuracy_m,
        cell_type: cell.cell_type,
        cell_key: cell.cell_key.clone(),
        network_name: cell.network_name.clone(),
        capabilities: cell.capabilities.clone(),
        frequency: cell.frequency,
        rssi: cell.rssi,
        uploaded: false,
    })
}

/// Enrich one probe request observation from a reassembled probe batch.
pub fn enrich_probe_observation(
    state: &mut EnrichmentState,
    session_id: i64,
    hdr_scan_end_ms: u32,
    hdr_scan_end_unix_ms: u64,
    sniffer_id: u32,
    wire: &wardriver_proto::ProbeObservationWire,
) -> EnrichOutcome {
    if let Some(reason) = csv_enqueue_block_reason(state) {
        return EnrichOutcome::Dropped(reason);
    }

    // Convert SSID bytes to string (handle non-UTF-8)
    let ssid = String::from_utf8_lossy(&wire.ssid[..wire.ssid_len as usize])
        .trim()
        .to_string();

    // Use scan_end as time anchor for all entries in batch
    let obs_mono_ms = hdr_scan_end_ms;
    let Some(obs_unix) = gps_anchor::observed_unix_ms_from_anchor(
        hdr_scan_end_ms,
        hdr_scan_end_unix_ms,
        obs_mono_ms,
    ) else {
        return EnrichOutcome::Dropped("no_time_anchor");
    };

    let Some(fix) = gps_anchor_mobile::match_mobile_gps_fix_for_obs(
        &state.gps_hist,
        obs_unix,
        MAX_FIX_TIMELINE_DELTA_MS,
    ) else {
        return EnrichOutcome::Dropped("no_gps_match");
    };

    // Dedup: separate HashMap keyed on (src_mac, ssid)
    let dedup_key = (wire.src_mac, ssid.clone());
    let allow = state.probe_mac_table.allow(
        &dedup_key,
        fix.lat,
        fix.lon,
        REVISIT_RADIUS_M,
        Some(wire.rssi),
    );
    if !allow.allow_row {
        return EnrichOutcome::Dropped("probe_dedup");
    }

    let id = state.next_id;
    state.next_id += 1;
    state
        .probe_mac_table
        .update_on_log(&dedup_key, fix.lat, fix.lon, wire.rssi);

    EnrichOutcome::StoredProbe(ProbeObservation {
        id,
        session_id,
        sniffer_id,
        observed_unix_ms: obs_unix,
        lat: fix.lat,
        lon: fix.lon,
        altitude_m: fix.altitude_m.map(|a| a.round() as i32),
        accuracy_m: fix.accuracy_m,
        src_mac: wire.src_mac,
        ssid,
        channel: wire.channel,
        rssi: wire.rssi,
    })
}
