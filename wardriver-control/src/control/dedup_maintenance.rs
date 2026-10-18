//! Dedup / reassembly table maintenance for the control main loop.

use std::collections::HashMap;
use std::hash::Hash;

use indexmap::IndexMap;

use super::espnow_rx::{self, DedupKey, Reasm, ReasmBle, ReasmKey};
use crate::csv;
use wardriver_shared::memory_policy::RuntimeMemoryPolicy;
use wardriver_stats as stats;

/// Frag-dedup cap. Sized for ~7 sniffers × ~5 frags/s × 60 s window with headroom.
/// Eviction is LRU (by insertion order, which equals first-seen order); see
/// [`prune_recent_maps`].
pub(crate) const DEDUP_MAX_ENTRIES: usize = 4096;
const DEDUP_MAX_AGE_MS: u64 = 60_000;

/// When free heap falls below this, shrink WiGLE deduper tables (LRU emergency relief).
pub(crate) const ROW_DEDUP_HEAP_TRIM_BYTES: u32 = 32_000;

/// Evict oldest dedup entries by insertion order until under cap.
///
/// First evicts entries older than [`DEDUP_MAX_AGE_MS`] (still cheap with `IndexMap`'s
/// front-removal). If still over cap, evicts the oldest 25 % so we don't churn on every
/// fragment when the table is consistently saturated.
pub(crate) fn prune_key_lru<K: Eq + Hash>(
    dedup: &mut IndexMap<K, u64>,
    now_ms: u64,
    max_entries: usize,
    budget: usize,
) {
    if dedup.len() <= max_entries {
        return;
    }
    let mut work: usize = 0;
    while let Some((_, seen_ms)) = dedup.get_index(0) {
        if work >= budget {
            break;
        }
        if now_ms.saturating_sub(*seen_ms) <= DEDUP_MAX_AGE_MS {
            break;
        }
        if dedup.shift_remove_index(0).is_none() {
            break;
        }
        work += 1;
    }
    if dedup.len() > max_entries {
        let target = (max_entries * 3) / 4;
        while dedup.len() > target && work < budget {
            if dedup.shift_remove_index(0).is_none() {
                break;
            }
            work += 1;
        }
    }
}

pub(crate) fn prune_recent_maps(
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, Reasm>,
    now_ms: u64,
    policy: &RuntimeMemoryPolicy,
) {
    prune_key_lru(
        dedup,
        now_ms,
        policy.dedup_max_entries.max(DEDUP_MAX_ENTRIES),
        policy.maintenance_items_per_tick,
    );
    espnow_rx::prune_reasm_map(reasm, now_ms, policy);
}

pub(crate) fn prune_recent_maps_ble(
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, ReasmBle>,
    now_ms: u64,
    policy: &RuntimeMemoryPolicy,
) {
    prune_key_lru(
        dedup,
        now_ms,
        policy.dedup_max_entries.max(DEDUP_MAX_ENTRIES),
        policy.maintenance_items_per_tick,
    );
    espnow_rx::prune_reasm_ble_map(reasm, now_ms, policy);
}

pub(crate) fn maybe_trim_mac_table(table: &mut csv::MacTable, policy: &RuntimeMemoryPolicy) {
    let trim_floor = policy
        .row_dedup_heap_trim_bytes
        .max(ROW_DEDUP_HEAP_TRIM_BYTES);
    if stats::free_heap_bytes() < trim_floor || stats::free_internal_heap_bytes() < trim_floor / 2 {
        table.shrink_to_cap_fraction(0.5);
    }
}
