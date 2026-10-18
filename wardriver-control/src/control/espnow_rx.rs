//! ESP-NOW RX prebuffer drain and scan-fragment reassembly state.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use indexmap::IndexMap;

use wardriver_core::scan_reasm::{
    self, ingest_ble_scan_frag_with_extras as core_ingest_ble_with_extras,
    ingest_probe_observation_frag as core_ingest_probe,
    ingest_wifi_scan_frag_with_extras as core_ingest_wifi_with_extras, ReasmBle as CoreReasmBle,
    ReasmProbe as CoreReasmProbe, ReasmWifi,
};
use wardriver_proto::{
    self as proto, BleEntryV1, BleScanFragV1Header, ProbeObservationFragV1Header,
    ProbeObservationWire, ScanEntryExtras, ScanFragV1Header,
};
use wardriver_shared::memory_policy::RuntimeMemoryPolicy;
use wardriver_stats as stats;

pub type ReasmKey = scan_reasm::ReasmKey;
pub type DedupKey = scan_reasm::DedupKey;

pub const REASM_MAX_ENTRIES: usize = 128;
pub const REASM_MAX_AGE_MS: u64 = 30_000;

/// WiFi reassembly state (canonical layout from `wardriver-core::scan_reasm`).
pub type Reasm = ReasmWifi;
pub type ReasmBle = CoreReasmBle;
pub type ReasmProbe = CoreReasmProbe;

/// Lookup dashboard sniffer id for an ESP-NOW peer MAC.
pub fn sniffer_id_for_mac(mac: &[u8; 6]) -> Option<u32> {
    stats::sniffer_id_for_mac(mac)
}

/// True when `mac`'s sniffer still has in-flight wifi or BLE reassembly entries.
pub fn has_incomplete_reasm_for_mac(
    mac: &[u8; 6],
    reasm: &HashMap<ReasmKey, Reasm>,
    reasm_ble: &HashMap<ReasmKey, ReasmBle>,
) -> bool {
    let Some(sid) = sniffer_id_for_mac(mac) else {
        return false;
    };
    reasm.keys().any(|k| k.0 == sid) || reasm_ble.keys().any(|k| k.0 == sid)
}

fn evict_oldest_reasm_entries<T, F>(
    reasm: &mut HashMap<ReasmKey, T>,
    now_ms: u64,
    max_entries: usize,
    budget: usize,
    updated_ms: F,
) where
    F: Fn(&T) -> u64,
{
    if reasm.len() <= max_entries {
        return;
    }
    reasm.retain(|_, entry| now_ms.saturating_sub(updated_ms(entry)) <= REASM_MAX_AGE_MS);
    if reasm.len() <= max_entries {
        return;
    }
    let mut items: Vec<(ReasmKey, u64)> = reasm.iter().map(|(k, v)| (*k, updated_ms(v))).collect();
    items.sort_unstable_by_key(|(_, ts)| *ts);
    let mut removed = 0usize;
    for (k, _) in items {
        if reasm.len() <= max_entries || removed >= budget {
            break;
        }
        if reasm.remove(&k).is_some() {
            removed += 1;
        }
    }
    if removed > 0 {
        stats::REASM_OVERFLOW_EVICTIONS.fetch_add(removed as u32, Ordering::Relaxed);
    }
}

pub fn prune_reasm_map(
    reasm: &mut HashMap<ReasmKey, Reasm>,
    now_ms: u64,
    policy: &RuntimeMemoryPolicy,
) {
    evict_oldest_reasm_entries(
        reasm,
        now_ms,
        policy.reasm_max_entries.max(REASM_MAX_ENTRIES),
        policy.maintenance_items_per_tick,
        |e| e.updated_ms,
    );
}

pub fn prune_reasm_ble_map(
    reasm: &mut HashMap<ReasmKey, ReasmBle>,
    now_ms: u64,
    policy: &RuntimeMemoryPolicy,
) {
    evict_oldest_reasm_entries(
        reasm,
        now_ms,
        policy.reasm_max_entries.max(REASM_MAX_ENTRIES),
        policy.maintenance_items_per_tick,
        |e| e.updated_ms,
    );
}

/// Dedup + reassemble one WiFi scan fragment; returns completed batch with TLV sidecar.
pub fn ingest_wifi_scan_frag_with_extras(
    hdr: ScanFragV1Header,
    aps: Vec<proto::ApEntryV1>,
    extras: Vec<ScanEntryExtras>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, Reasm>,
    now: u64,
) -> Option<(
    ScanFragV1Header,
    Vec<proto::ApEntryV1>,
    Vec<ScanEntryExtras>,
)> {
    let before = dedup.len();
    let out = core_ingest_wifi_with_extras(hdr, aps, extras, dedup, reasm, now);
    if dedup.len() > before {
        stats::CONTROL_STAGE_DEDUP.fetch_add(1, Ordering::Relaxed);
    }
    if out.is_some() {
        stats::CONTROL_STAGE_REASM.fetch_add(1, Ordering::Relaxed);
    }
    out
}

/// Dedup + reassemble one BLE scan fragment with TLV sidecar rows.
pub fn ingest_ble_scan_frag_with_extras(
    hdr: BleScanFragV1Header,
    entries: Vec<BleEntryV1>,
    extras: Vec<ScanEntryExtras>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm_ble: &mut HashMap<ReasmKey, ReasmBle>,
    now: u64,
) -> Option<(BleScanFragV1Header, ReasmBle)> {
    let before = dedup.len();
    let out = core_ingest_ble_with_extras(hdr, entries, extras, dedup, reasm_ble, now);
    if dedup.len() > before {
        stats::CONTROL_STAGE_DEDUP.fetch_add(1, Ordering::Relaxed);
    }
    if out.is_some() {
        stats::CONTROL_STAGE_REASM.fetch_add(1, Ordering::Relaxed);
    }
    out
}

/// Dedup + reassemble one probe observation fragment; returns completed batch when ready.
pub fn ingest_probe_observation_frag(
    hdr: ProbeObservationFragV1Header,
    entries: Vec<ProbeObservationWire>,
    dedup_probe: &mut IndexMap<DedupKey, u64>,
    reasm_probe: &mut HashMap<ReasmKey, ReasmProbe>,
    now: u64,
) -> Option<(ProbeObservationFragV1Header, Vec<ProbeObservationWire>)> {
    let before = dedup_probe.len();
    let out = core_ingest_probe(hdr, entries, dedup_probe, reasm_probe, now);
    if dedup_probe.len() > before {
        stats::CONTROL_STAGE_DEDUP.fetch_add(1, Ordering::Relaxed);
    }
    if out.is_some() {
        stats::CONTROL_STAGE_REASM.fetch_add(1, Ordering::Relaxed);
    }
    out
}

pub fn drop_reasm_for_sniffer_with_probe(
    sid: u32,
    reasm: &mut HashMap<ReasmKey, Reasm>,
    reasm_ble: &mut HashMap<ReasmKey, ReasmBle>,
    reasm_probe: &mut HashMap<ReasmKey, ReasmProbe>,
) {
    reasm.retain(|k, _| k.0 != sid);
    reasm_ble.retain(|k, _| k.0 != sid);
    reasm_probe.retain(|k, _| k.0 != sid);
}
