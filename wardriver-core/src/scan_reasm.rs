//! Host-testable ESP-NOW scan-fragment dedup and reassembly (WiFi + BLE).
//!
//! Mirrors control [`wardriver-control/src/control/espnow_rx.rs`] logic without stats
//! or memory-policy eviction — suitable for pipeline integration tests.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use wardriver_proto::{
    ApEntryV1, BleEntryV1, BleScanFragV1Header, ProbeObservationFragV1Header, ProbeObservationWire,
    ScanEntryExtras, ScanFragV1Header, AP_ENTRY_WIRE_OVERHEAD, PROBE_OBSERVATION_WIRE_SIZE,
    SCAN_FRAG_ENTRY_BUDGET,
};

pub type ReasmKey = (u32, u32, u32);
pub type DedupKey = (u32, u32, u32, u8);

/// Pre-reserve WiFi reassembly `Vec` capacity (same formula as control firmware).
fn scan_reasm_aps_reserve(frag_total: u8) -> usize {
    let max_hidden_per_frag = SCAN_FRAG_ENTRY_BUDGET / AP_ENTRY_WIRE_OVERHEAD;
    (frag_total as usize).saturating_mul(max_hidden_per_frag)
}

/// In-progress WiFi scan batch reassembly state.
#[derive(Debug, Clone)]
pub struct ReasmWifi {
    pub frag_total: u8,
    pub got: HashSet<u8>,
    pub aps: Vec<ApEntryV1>,
    pub extras: Vec<ScanEntryExtras>,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_unix_ms: i64,
    pub updated_ms: u64,
}

/// In-progress BLE scan batch reassembly state.
#[derive(Debug, Clone)]
pub struct ReasmBle {
    pub frag_total: u8,
    pub got: HashSet<u8>,
    pub entries: Vec<BleEntryV1>,
    pub extras: Vec<ScanEntryExtras>,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_unix_ms: i64,
    pub updated_ms: u64,
}

/// In-progress probe observation batch reassembly state.
#[derive(Debug, Clone)]
pub struct ReasmProbe {
    pub frag_total: u8,
    pub got: HashSet<u8>,
    pub entries: Vec<ProbeObservationWire>,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_unix_ms: i64,
    pub updated_ms: u64,
}

/// Reassembly-state invariants shared by every scan-fragment batch.
trait ReasmState {
    fn frag_total(&self) -> u8;
    fn sniffer_boot_id(&self) -> u32;
    fn sniffer_scan_end_unix_ms(&self) -> i64;
    fn got(&mut self) -> &mut HashSet<u8>;
    fn updated_ms(&mut self) -> &mut u64;
}

impl ReasmState for ReasmWifi {
    fn frag_total(&self) -> u8 {
        self.frag_total
    }
    fn sniffer_boot_id(&self) -> u32 {
        self.sniffer_boot_id
    }
    fn sniffer_scan_end_unix_ms(&self) -> i64 {
        self.sniffer_scan_end_unix_ms
    }
    fn got(&mut self) -> &mut HashSet<u8> {
        &mut self.got
    }
    fn updated_ms(&mut self) -> &mut u64 {
        &mut self.updated_ms
    }
}

impl ReasmState for ReasmBle {
    fn frag_total(&self) -> u8 {
        self.frag_total
    }
    fn sniffer_boot_id(&self) -> u32 {
        self.sniffer_boot_id
    }
    fn sniffer_scan_end_unix_ms(&self) -> i64 {
        self.sniffer_scan_end_unix_ms
    }
    fn got(&mut self) -> &mut HashSet<u8> {
        &mut self.got
    }
    fn updated_ms(&mut self) -> &mut u64 {
        &mut self.updated_ms
    }
}

impl ReasmState for ReasmProbe {
    fn frag_total(&self) -> u8 {
        self.frag_total
    }
    fn sniffer_boot_id(&self) -> u32 {
        self.sniffer_boot_id
    }
    fn sniffer_scan_end_unix_ms(&self) -> i64 {
        self.sniffer_scan_end_unix_ms
    }
    fn got(&mut self) -> &mut HashSet<u8> {
        &mut self.got
    }
    fn updated_ms(&mut self) -> &mut u64 {
        &mut self.updated_ms
    }
}

/// Shared scan-fragment dedup + reassembly kernel (WiFi / BLE / probe).
///
/// `len_ok` covers the per-family length guard; `capacity`/`build`/`on_append`/`on_complete`
/// encode the per-family differences (capacity formula, extras presence, return shape).
/// Behavior is byte-identical to the prior three hand-rolled fns.
fn reassemble_scan_frag<E, X, R, C>(
    dk: DedupKey,
    rk: ReasmKey,
    frag_total: u8,
    frag_idx: u8,
    sniffer_boot_id: u32,
    sniffer_scan_end_unix_ms: u64,
    entries: &E,
    extras: Option<&X>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, R>,
    now: u64,
    len_ok: bool,
    capacity: impl Fn() -> usize,
    build: impl Fn(u8, u32, u64, usize) -> R,
    on_append: impl Fn(&mut R, &E, Option<&X>),
    on_complete: impl Fn(R) -> C,
) -> Option<C>
where
    R: ReasmState,
{
    if !len_ok {
        return None;
    }
    if frag_total == 0 || frag_idx >= frag_total {
        return None;
    }
    if dedup.insert(dk, now).is_some() {
        return None;
    }

    let mut inconsistent = false;
    let complete = match reasm.entry(rk) {
        Entry::Occupied(mut occ) => {
            let e = occ.get_mut();
            if e.frag_total() != frag_total {
                inconsistent = true;
                false
            } else if e.sniffer_boot_id() != sniffer_boot_id {
                let cap = capacity();
                *e = build(frag_total, sniffer_boot_id, sniffer_scan_end_unix_ms, cap);
                e.got().insert(frag_idx);
                on_append(e, entries, extras);
                e.got().len() == frag_total as usize
            } else if e.sniffer_scan_end_unix_ms() != sniffer_scan_end_unix_ms as i64 {
                inconsistent = true;
                false
            } else {
                *e.updated_ms() = now;
                if e.got().insert(frag_idx) {
                    on_append(e, entries, extras);
                }
                e.got().len() == frag_total as usize
            }
        }
        Entry::Vacant(vac) => {
            let cap = capacity();
            let e = vac.insert(build(
                frag_total,
                sniffer_boot_id,
                sniffer_scan_end_unix_ms,
                cap,
            ));
            e.got().insert(frag_idx);
            on_append(e, entries, extras);
            e.got().len() == frag_total as usize
        }
    };
    if inconsistent {
        reasm.remove(&rk);
        return None;
    }
    if !complete {
        return None;
    }
    reasm.remove(&rk).map(|entry| on_complete(entry))
}

/// Dedup + reassemble one WiFi scan fragment; returns completed batch when all frags arrive.
pub fn ingest_wifi_scan_frag(
    hdr: ScanFragV1Header,
    aps: Vec<ApEntryV1>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, ReasmWifi>,
    now: u64,
) -> Option<(ScanFragV1Header, Vec<ApEntryV1>)> {
    let extras = vec![ScanEntryExtras::default(); aps.len()];
    ingest_wifi_scan_frag_with_extras(hdr, aps, extras, dedup, reasm, now)
        .map(|(h, aps, _extras)| (h, aps))
}

/// WiFi reassembly retaining per-AP TLV sidecar rows.
pub fn ingest_wifi_scan_frag_with_extras(
    hdr: ScanFragV1Header,
    aps: Vec<ApEntryV1>,
    extras: Vec<ScanEntryExtras>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, ReasmWifi>,
    now: u64,
) -> Option<(ScanFragV1Header, Vec<ApEntryV1>, Vec<ScanEntryExtras>)> {
    reassemble_scan_frag(
        (hdr.sniffer_id, hdr.scan_batch_id, hdr.seq, hdr.frag_idx),
        (hdr.sniffer_id, hdr.scan_batch_id, hdr.seq),
        hdr.frag_total,
        hdr.frag_idx,
        hdr.sniffer_boot_id,
        hdr.sniffer_scan_end_unix_ms,
        &aps,
        Some(&extras),
        dedup,
        reasm,
        now,
        aps.len() == extras.len(),
        || scan_reasm_aps_reserve(hdr.frag_total),
        |frag_total, boot, end, cap| ReasmWifi {
            frag_total,
            got: HashSet::with_capacity(frag_total as usize),
            aps: Vec::with_capacity(cap),
            extras: Vec::with_capacity(cap),
            sniffer_boot_id: boot,
            sniffer_scan_end_unix_ms: end as i64,
            updated_ms: now,
        },
        |e, ents, x| {
            e.aps.extend_from_slice(ents);
            if let Some(x) = x {
                e.extras.extend_from_slice(x);
            }
        },
        |entry| (hdr, entry.aps, entry.extras),
    )
}

/// Dedup + reassemble one BLE scan fragment; returns completed batch when all frags arrive.
pub fn ingest_ble_scan_frag(
    hdr: BleScanFragV1Header,
    entries: Vec<BleEntryV1>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm_ble: &mut HashMap<ReasmKey, ReasmBle>,
    now: u64,
) -> Option<(BleScanFragV1Header, ReasmBle)> {
    let extras = vec![ScanEntryExtras::default(); entries.len()];
    ingest_ble_scan_frag_with_extras(hdr, entries, extras, dedup, reasm_ble, now)
}

/// BLE reassembly retaining per-entry TLV sidecar rows.
pub fn ingest_ble_scan_frag_with_extras(
    hdr: BleScanFragV1Header,
    entries: Vec<BleEntryV1>,
    extras: Vec<ScanEntryExtras>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm_ble: &mut HashMap<ReasmKey, ReasmBle>,
    now: u64,
) -> Option<(BleScanFragV1Header, ReasmBle)> {
    reassemble_scan_frag(
        (hdr.sniffer_id, hdr.scan_batch_id, hdr.seq, hdr.frag_idx),
        (hdr.sniffer_id, hdr.scan_batch_id, hdr.seq),
        hdr.frag_total,
        hdr.frag_idx,
        hdr.sniffer_boot_id,
        hdr.sniffer_scan_end_unix_ms,
        &entries,
        Some(&extras),
        dedup,
        reasm_ble,
        now,
        entries.len() == extras.len(),
        || {
            (hdr.frag_total as usize)
                .saturating_mul(hdr.entry_count as usize)
                .max(hdr.entry_count as usize)
        },
        |frag_total, boot, end, cap| ReasmBle {
            frag_total,
            got: HashSet::with_capacity(frag_total as usize),
            entries: Vec::with_capacity(cap),
            extras: Vec::with_capacity(cap),
            sniffer_boot_id: boot,
            sniffer_scan_end_unix_ms: end as i64,
            updated_ms: now,
        },
        |e, ents, x| {
            e.entries.extend_from_slice(ents);
            if let Some(x) = x {
                e.extras.extend_from_slice(x);
            }
        },
        |entry| (hdr, entry),
    )
}

/// Dedup + reassemble one probe observation fragment; returns completed batch when all frags arrive.
pub fn ingest_probe_observation_frag(
    hdr: ProbeObservationFragV1Header,
    entries: Vec<ProbeObservationWire>,
    dedup: &mut IndexMap<DedupKey, u64>,
    reasm: &mut HashMap<ReasmKey, ReasmProbe>,
    now: u64,
) -> Option<(ProbeObservationFragV1Header, Vec<ProbeObservationWire>)> {
    reassemble_scan_frag(
        (hdr.sniffer_id, hdr.scan_batch_id, hdr.seq, hdr.frag_idx),
        (hdr.sniffer_id, hdr.scan_batch_id, hdr.seq),
        hdr.frag_total,
        hdr.frag_idx,
        hdr.sniffer_boot_id,
        hdr.sniffer_scan_end_unix_ms,
        &entries,
        None::<&ScanEntryExtras>,
        dedup,
        reasm,
        now,
        true,
        || {
            (hdr.frag_total as usize)
                .saturating_mul(hdr.entry_count as usize)
                .saturating_mul(PROBE_OBSERVATION_WIRE_SIZE)
        },
        |frag_total, boot, end, cap| ReasmProbe {
            frag_total,
            got: HashSet::with_capacity(frag_total as usize),
            entries: Vec::with_capacity(cap),
            sniffer_boot_id: boot,
            sniffer_scan_end_unix_ms: end as i64,
            updated_ms: now,
        },
        |e, ents, _| e.entries.extend_from_slice(ents),
        |entry| (hdr, entry.entries),
    )
}
