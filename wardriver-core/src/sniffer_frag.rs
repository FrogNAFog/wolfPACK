//! ESP-NOW send frag counts for sniffer bulk transport (host-testable).

use wardriver_proto::{self as proto, ApEntryV1, BleEntryV1, ProbeReqBurstV1, ScanEntryExtras};

/// Max AP rows per ESP-NOW reassembly group (`ScanFragV1Header::frag_total` is `u8`).
pub const MAX_APS_PER_REASSEMBLY: usize = proto::MAX_APS_PER_FRAG_WORST_CASE * u8::MAX as usize;

/// Split AP rows into reassembly groups that fit `frag_total <= 255`.
pub fn wifi_reassembly_ranges(
    aps: &[ApEntryV1],
    extras: &[ScanEntryExtras],
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < aps.len() {
        let max_end = (offset + MAX_APS_PER_REASSEMBLY).min(aps.len());
        let mut lo = offset + 1;
        let mut hi = max_end;
        let mut best = lo;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let nchunks = proto::count_scan_frag_chunks_with_extras(
                &aps[offset..mid],
                extras_slice(extras, offset, mid),
            );
            if nchunks > 0 && nchunks <= u8::MAX as usize {
                best = mid;
                lo = mid.saturating_add(1);
            } else if mid == offset + 1 {
                best = mid;
                break;
            } else {
                hi = mid.saturating_sub(1);
            }
        }
        out.push((offset, best));
        offset = best;
    }
    out
}

fn extras_slice(extras: &[ScanEntryExtras], start: usize, end: usize) -> &[ScanEntryExtras] {
    if extras.len() >= end {
        &extras[start..end]
    } else if extras.len() > start {
        &extras[start..]
    } else {
        &[]
    }
}

pub fn count_wifi_send_frags_with_extras(
    wifi_batches: &[(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)],
) -> u32 {
    let mut total = 0u32;
    for (_, _, aps, extras) in wifi_batches {
        if aps.is_empty() {
            continue;
        }
        for (start, end) in wifi_reassembly_ranges(aps, extras) {
            total = total.saturating_add(proto::count_scan_frag_chunks_with_extras(
                &aps[start..end],
                extras_slice(extras, start, end),
            ) as u32);
        }
    }
    total
}

pub fn count_probe_send_frags(probe_groups: &[(u32, u64, Vec<ProbeReqBurstV1>)]) -> u32 {
    probe_groups
        .iter()
        .map(|(_, _, bursts)| {
            let n = bursts.len();
            if n == 0 {
                0
            } else {
                ((n + proto::MAX_PROBE_REQ_PER_FRAG - 1) / proto::MAX_PROBE_REQ_PER_FRAG) as u32
            }
        })
        .sum()
}

pub fn count_ble_send_frags_with_extras(entries: &[BleEntryV1], extras: &[ScanEntryExtras]) -> u32 {
    if entries.is_empty() {
        return 0;
    }
    let frag_total = proto::chunk_ble_scan_frag_ranges(entries, extras).len();
    if frag_total > u8::MAX as usize {
        return 0;
    }
    frag_total as u32
}
