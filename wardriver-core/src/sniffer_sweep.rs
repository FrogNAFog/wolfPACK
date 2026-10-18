//! Pure sniffer sweep transforms and outbound bulk grouping (host-testable).

use std::collections::{HashMap, HashSet};

use wardriver_proto::{
    ApEntryV1, ProbeReqBurstV1, ScanEntryExtras, AP_ENTRY_SIZE, BLE_ENTRY_SIZE,
    PROBE_REQ_ENTRY_SIZE,
};

/// One WiFi AP row with the mono/unix anchors from its dwell sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundWifiRow {
    pub scan_end_ms: u32,
    pub scan_end_unix_ms: u64,
    pub entry: ApEntryV1,
    pub extras: ScanEntryExtras,
}

/// Min gap between queued WiFi rows for the same BSSID within one off-control hop (~2 Hz without GPS).
pub const BSSID_SEND_MIN_INTERVAL_MS: u32 = 500;

const BLE_ANCHOR_BYTES: usize = 4 + 8;

/// Merge per-channel [`ProbeReqBurstV1`] rows by source MAC so the control node receives one
/// row per MAC per sweep (reduces ESP-NOW reordering across channel frames).
pub fn merge_probe_bursts_by_src(
    groups: Vec<(u32, u64, Vec<ProbeReqBurstV1>)>,
) -> Vec<(u32, u64, Vec<ProbeReqBurstV1>)> {
    if groups.is_empty() {
        return groups;
    }
    let mut scan_end_ms_max: u32 = 0;
    let mut anchor_unix: u64 = 0;
    let mut map: HashMap<[u8; 6], ProbeReqBurstV1> = HashMap::new();
    for (scan_end_ms, scan_end_unix_ms, bursts) in groups {
        if scan_end_ms >= scan_end_ms_max {
            scan_end_ms_max = scan_end_ms;
            anchor_unix = scan_end_unix_ms;
        }
        for b in bursts {
            map.entry(b.src)
                .and_modify(|acc| merge_probe_burst_into(acc, &b))
                .or_insert(b);
        }
    }
    let mut merged: Vec<ProbeReqBurstV1> = map.into_values().collect();
    merged.sort_by_key(|b| b.last_seen_ms32);
    vec![(scan_end_ms_max, anchor_unix, merged)]
}

/// Wardrive probe batches are follow-up telemetry for MACs that raised DetectionAlert since boot.
pub fn filter_probe_groups_by_alerted_srcs(
    groups: Vec<(u32, u64, Vec<ProbeReqBurstV1>)>,
    alerted: &HashSet<[u8; 6]>,
) -> Vec<(u32, u64, Vec<ProbeReqBurstV1>)> {
    groups
        .into_iter()
        .filter_map(|(scan_end_ms, scan_end_unix_ms, bursts)| {
            let filtered: Vec<_> = bursts
                .into_iter()
                .filter(|b| alerted.contains(&b.src))
                .collect();
            if filtered.is_empty() {
                None
            } else {
                Some((scan_end_ms, scan_end_unix_ms, filtered))
            }
        })
        .collect()
}

/// At most one row per BSSID per `min_interval_ms` window; within a window keep strongest RSSI.
/// Uses `obs_mono_ms` (wrapping u32); window anchor is the first observation in each window.
/// Keeps the winning observation's TLV sidecar.
pub fn throttle_wifi_entries_with_extras(
    entries: Vec<ApEntryV1>,
    mut extras: Vec<ScanEntryExtras>,
    min_interval_ms: u32,
) -> (Vec<ApEntryV1>, Vec<ScanEntryExtras>) {
    extras.resize(entries.len(), ScanEntryExtras::default());
    let mut window_start_ms: HashMap<[u8; 6], u32> = HashMap::new();
    let mut window_best_idx: HashMap<[u8; 6], usize> = HashMap::new();
    let mut out: Vec<(ApEntryV1, ScanEntryExtras)> = Vec::with_capacity(entries.len());
    for (e, ex) in entries.into_iter().zip(extras) {
        match window_start_ms.get(&e.bssid) {
            None => {
                window_start_ms.insert(e.bssid, e.obs_mono_ms);
                window_best_idx.insert(e.bssid, out.len());
                out.push((e, ex));
            }
            Some(&start) if e.obs_mono_ms.wrapping_sub(start) >= min_interval_ms => {
                window_start_ms.insert(e.bssid, e.obs_mono_ms);
                window_best_idx.insert(e.bssid, out.len());
                out.push((e, ex));
            }
            Some(_) => {
                let idx = window_best_idx[&e.bssid];
                let cur = &out[idx].0;
                if e.rssi > cur.rssi || (e.rssi == cur.rssi && e.obs_mono_ms > cur.obs_mono_ms) {
                    out[idx] = (e, ex);
                }
            }
        }
    }
    out.into_iter().unzip()
}

/// Concat per-dwell WiFi rows for transport; no BSSID/RSSI collapse (WiGLE trilateration).
pub fn merge_wifi_sweep_groups(
    groups: Vec<(u32, u64, Vec<ApEntryV1>)>,
) -> Option<(u32, u64, Vec<ApEntryV1>)> {
    if groups.is_empty() {
        return None;
    }
    let mut scan_end_ms_max: u32 = 0;
    let mut anchor_unix: u64 = 0;
    let mut all_entries: Vec<ApEntryV1> = Vec::new();
    for (scan_end_ms, scan_end_unix_ms, entries) in groups {
        if scan_end_ms >= scan_end_ms_max {
            scan_end_ms_max = scan_end_ms;
            anchor_unix = scan_end_unix_ms;
        }
        all_entries.extend(entries);
    }
    Some((scan_end_ms_max, anchor_unix, all_entries))
}

/// True when the sweep captured WiFi AP rows and/or probe bursts for wardrive bulk TX.
pub fn had_wifi_bulk(
    groups: &[(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)],
    probe_groups: &[(u32, u64, Vec<ProbeReqBurstV1>)],
) -> bool {
    groups.iter().any(|(_, _, aps, _)| !aps.is_empty())
        || probe_groups.iter().any(|(_, _, bursts)| !bursts.is_empty())
}

fn merge_probe_burst_into(acc: &mut ProbeReqBurstV1, b: &ProbeReqBurstV1) {
    let w1 = acc.wildcard_count as u16;
    let w2 = b.wildcard_count as u16;
    acc.wildcard_count = w1.saturating_add(w2).min(u16::from(u8::MAX)) as u8;
    if b.last_seen_ms32 >= acc.last_seen_ms32 {
        acc.channel = b.channel;
    }
    acc.first_seen_ms32 = acc.first_seen_ms32.min(b.first_seen_ms32);
    acc.last_seen_ms32 = acc.last_seen_ms32.max(b.last_seen_ms32);
    acc.rssi_min = acc.rssi_min.min(b.rssi_min);
    acc.rssi_max = acc.rssi_max.max(b.rssi_max);
    let n = (w1 as i32 + w2 as i32).max(1);
    let sum = acc.rssi_avg_q8 as i32 * w1 as i32 + b.rssi_avg_q8 as i32 * w2 as i32;
    acc.rssi_avg_q8 = (sum / n).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    if b.primary_sig_matched != 0 {
        acc.primary_sig_matched = 1;
    }
}

/// Host-friendly probe burst summary (mirrors firmware `ProbeReqBurstLocal`).
#[derive(Clone, Copy, Debug)]
pub struct ProbeReqBurstInput {
    pub src: [u8; 6],
    pub wildcard_count: u16,
    pub channel: u8,
    pub first_seen_ms32: u32,
    pub last_seen_ms32: u32,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub primary_sig_matched: bool,
}

/// Convert local promisc probe summaries to wire rows (capped per channel).
pub fn local_probe_bursts_to_wire(probe_bursts: Vec<ProbeReqBurstInput>) -> Vec<ProbeReqBurstV1> {
    let mut out: Vec<ProbeReqBurstV1> = Vec::with_capacity(probe_bursts.len().min(32));
    for b in probe_bursts {
        if b.wildcard_count == 0 {
            continue;
        }
        out.push(ProbeReqBurstV1 {
            src: b.src,
            wildcard_count: (b.wildcard_count.min(u16::from(u8::MAX))) as u8,
            channel: b.channel,
            rssi_min: b.rssi_min,
            rssi_max: b.rssi_max,
            rssi_avg_q8: b.rssi_avg_q8,
            first_seen_ms32: b.first_seen_ms32,
            last_seen_ms32: b.last_seen_ms32,
            primary_sig_matched: if b.primary_sig_matched { 1 } else { 0 },
        });
        if out.len() >= 32 {
            break;
        }
    }
    out
}

pub fn lane_pressure(used: usize, budget: usize) -> bool {
    used >= budget.saturating_mul(9) / 10
}

pub fn wifi_row_charge_bytes() -> usize {
    4 + 8 + AP_ENTRY_SIZE
}

pub fn probe_row_charge_bytes() -> usize {
    4 + 8 + PROBE_REQ_ENTRY_SIZE
}

pub fn ble_lane_charge_bytes(entry_count: usize) -> usize {
    if entry_count == 0 {
        0
    } else {
        BLE_ANCHOR_BYTES + entry_count * BLE_ENTRY_SIZE
    }
}

pub fn wifi_batch_keys(rows: &[OutboundWifiRow]) -> Vec<(u32, u64)> {
    let mut seen = HashMap::new();
    for row in rows {
        seen.entry((row.scan_end_ms, row.scan_end_unix_ms))
            .or_insert(());
    }
    let mut keys: Vec<_> = seen.into_keys().collect();
    keys.sort_by_key(|(ms, _)| *ms);
    keys
}

pub fn probe_batch_keys(rows: &[(u32, u64, ProbeReqBurstV1)]) -> Vec<(u32, u64)> {
    let mut seen = HashMap::new();
    for (ms, unix, _) in rows {
        seen.entry((*ms, *unix)).or_insert(());
    }
    let mut keys: Vec<_> = seen.into_keys().collect();
    keys.sort_by_key(|(ms, _)| *ms);
    keys
}

pub fn oldest_wifi_batch(rows: &[OutboundWifiRow]) -> Option<(u32, u64)> {
    wifi_batch_keys(rows).into_iter().min_by_key(|(ms, _)| *ms)
}

pub fn oldest_probe_batch(rows: &[(u32, u64, ProbeReqBurstV1)]) -> Option<(u32, u64)> {
    probe_batch_keys(rows).into_iter().min_by_key(|(ms, _)| *ms)
}

pub fn group_wifi_rows(
    rows: &[OutboundWifiRow],
) -> Vec<(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)> {
    let mut order: Vec<(u32, u64)> = Vec::new();
    let mut map: HashMap<(u32, u64), (Vec<ApEntryV1>, Vec<ScanEntryExtras>)> = HashMap::new();
    for row in rows {
        let key = (row.scan_end_ms, row.scan_end_unix_ms);
        if !map.contains_key(&key) {
            order.push(key);
        }
        let slot = map.entry(key).or_default();
        slot.0.push(row.entry);
        slot.1.push(row.extras.clone());
    }
    order
        .into_iter()
        .filter_map(|key| {
            map.remove(&key)
                .map(|(entries, extras)| (key.0, key.1, entries, extras))
        })
        .collect()
}

pub fn group_probe_rows(
    rows: &[(u32, u64, ProbeReqBurstV1)],
) -> Vec<(u32, u64, Vec<ProbeReqBurstV1>)> {
    let mut order: Vec<(u32, u64)> = Vec::new();
    let mut map: HashMap<(u32, u64), Vec<ProbeReqBurstV1>> = HashMap::new();
    for (scan_end_ms, scan_end_unix_ms, burst) in rows {
        let key = (*scan_end_ms, *scan_end_unix_ms);
        if !map.contains_key(&key) {
            order.push(key);
        }
        map.entry(key).or_default().push(*burst);
    }
    order
        .into_iter()
        .filter_map(|key| map.remove(&key).map(|bursts| (key.0, key.1, bursts)))
        .collect()
}
