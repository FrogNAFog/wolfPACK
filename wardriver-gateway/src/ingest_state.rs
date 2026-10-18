//! Gateway ingest maps shared between ESP-NOW bridge and peer-pool eviction guards.

use std::collections::HashMap;

use indexmap::IndexMap;
use wardriver_core::scan_reasm::{DedupKey, ReasmBle, ReasmKey, ReasmProbe, ReasmWifi};
use wardriver_fleet_realtime::ingest::IngestPeerPoolState;
use wardriver_stats as stats;

/// WiFi + BLE scan reassembly state for gateway export path.
pub struct GatewayIngestState {
    pub reasm_dedup: IndexMap<DedupKey, u64>,
    pub reasm_state: HashMap<ReasmKey, ReasmWifi>,
    pub reasm_ble_dedup: IndexMap<DedupKey, u64>,
    pub reasm_ble_state: HashMap<ReasmKey, ReasmBle>,
    pub reasm_probe_dedup: IndexMap<DedupKey, u64>,
    pub reasm_probe_state: HashMap<ReasmKey, ReasmProbe>,
}

impl Default for GatewayIngestState {
    fn default() -> Self {
        Self {
            reasm_dedup: IndexMap::new(),
            reasm_state: HashMap::new(),
            reasm_ble_dedup: IndexMap::new(),
            reasm_ble_state: HashMap::new(),
            reasm_probe_dedup: IndexMap::new(),
            reasm_probe_state: HashMap::new(),
        }
    }
}

impl IngestPeerPoolState for GatewayIngestState {
    fn has_incomplete_reasm_for_mac(&self, mac: &[u8; 6]) -> bool {
        let Some(sid) = stats::sniffer_id_for_mac(mac) else {
            return false;
        };
        self.reasm_state.keys().any(|k| k.0 == sid)
            || self.reasm_ble_state.keys().any(|k| k.0 == sid)
            || self.reasm_probe_state.keys().any(|k| k.0 == sid)
    }

    fn drop_state_for_sniffer(&mut self, sniffer_id: u32) {
        self.reasm_dedup.retain(|k, _| k.0 != sniffer_id);
        self.reasm_state.retain(|k, _| k.0 != sniffer_id);
        self.reasm_ble_dedup.retain(|k, _| k.0 != sniffer_id);
        self.reasm_ble_state.retain(|k, _| k.0 != sniffer_id);
        self.reasm_probe_dedup.retain(|k, _| k.0 != sniffer_id);
        self.reasm_probe_state.retain(|k, _| k.0 != sniffer_id);
    }
}
