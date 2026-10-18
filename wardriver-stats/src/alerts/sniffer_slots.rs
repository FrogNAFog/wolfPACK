//! Sniffer table and internal hop aggregate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::counters::now_ms;

#[cfg(feature = "sniffer")]
/// Sniffer: ESP-NOW WiFi batch split because AP list exceeded one reassembly group (`u8` frag_total).
pub static SNIFFER_TRANSPORT_BATCHES_SPLIT: AtomicU32 = AtomicU32::new(0);

// -------- Internal hop aggregate (sniffer-only; feeds DwellAndSchedule timeout math) --------

#[derive(Clone, Copy, Default)]
// In-flight rolling-average WiFi set_channel hop aggregate (written by production
// sniffer_record_hop_timing; reader removed pending control-silence timeout wiring).
#[allow(dead_code)]
struct SnifferHopAgg {
    hop_count: u64,
    set_channel_us_sum: u64,
}

#[allow(dead_code)] // in-flight rolling-average aggregate (see SnifferHopAgg comment)
static SNIFFER_HOP_AGG: Mutex<SnifferHopAgg> = Mutex::new(SnifferHopAgg {
    hop_count: 0,
    set_channel_us_sum: 0,
});

#[derive(Clone, Copy, Debug, Default)]
pub struct SnifferSlot {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub last_seen_ms: u64,
    /// Completed WiFi [`wardriver_proto::ScanFrag`] batches received on control.
    pub wifi_frag_batches: u64,
    /// Completed BLE [`wardriver_proto::BleScanFrag`] batches received on control.
    pub ble_frag_batches: u64,
    pub ap_entries: u64,
    /// BLE observations assembled from [`wardriver_proto::BleScanFrag`] batches.
    pub ble_entries: u64,
    /// Completed probe observation [`wardriver_proto::ProbeObservationFrag`] batches received on control.
    pub probe_frag_batches: u64,
    /// Probe observations assembled from [`wardriver_proto::ProbeObservationFrag`] batches.
    pub probe_entries: u64,
    pub state: u8,
    pub ack_cmd_seq: u32,
    /// [`wardriver_proto::SnifferRole`] wire value from last ACK.
    pub role: u8,
    /// 1 if sniffer reports NimBLE-capable firmware.
    pub ble_capable: u8,
    /// Last ACK: 1 if sniffer reports non-DFS 5 GHz tune OK at boot.
    pub five_ghz_capable: u8, // NEW
    /// WiFi reassembly batches dropped (inconsistent frags).
    pub wifi_reasm_dropped: u64,
    /// BLE reassembly batches dropped (inconsistent frags).
    pub ble_reasm_dropped: u64,
    /// WiGLE rows enqueued from WiFi ingest (includes probe follow-ups).
    pub wifi_rows_enqueued: u64,
    /// WiGLE rows enqueued from BLE ingest.
    pub ble_rows_enqueued: u64,
    /// Last ACK: internal + SPIRAM free bytes (`heap_caps` sums).
    pub free_heap_bytes: u32,
    /// Last ACK: internal + SPIRAM total heap bytes.
    pub total_heap_bytes: u32,
    /// Last ACK: non-idle CPU load 0–100 (FreeRTOS run-time stats).
    pub cpu_load_pct: u8,
    /// PROTO cleanup candidate: mirrors `five_ghz_capable` (ACK wire field) until next PROTO bump.
    pub dfs_capable: u8,
    /// Operator-assigned band: 0 = unassigned, 1 = 2.4 GHz, 2 = 5 GHz.
    pub assigned_band: u8,
    /// Operator-pinned single channel (skips hopping). 0 means not pinned.
    pub pinned_channel: u8,
    /// Number of channels in the most recent `SetChannelSchedule` for this sniffer.
    pub assigned_chan_count: u8,
    /// Current schedule bucket (only first `assigned_chan_count` slots are meaningful).
    pub assigned_channels: [u8; wardriver_proto::CHANNELS_MAX],
    /// Last ACK `bulk_lost_cycles` when role is WiFi (sniffer-global counter; see ACK handler).
    pub wifi_bulk_lost_cycles: u32,
    /// Last ACK `bulk_lost_cycles` when role is BLE (sniffer-global counter; see ACK handler).
    pub ble_bulk_lost_cycles: u32,
    /// Last observed sniffer boot id (`0` = unknown). Changes across sniffer reboots.
    pub sniffer_boot_id: u32,
    /// Reboot transitions observed on control (boot id changed after a prior non-zero sighting).
    pub reboot_count: u32,
    /// Dashboard table slot (1..= [`crate::fleet::FLEET_MAX_SNIFFERS`]); 0 = unassigned.
    /// Roster adopt overwrites via [`set_adoption_adopted`]; else table index at first hello.
    pub fleet_slot: u8,
    /// Operator adoption: 0=empty, 1=Pending, 2=Adopted, 3=Removed.
    pub adoption_state: u8,
    /// Last completed dump: PeerJoin → PostDumpDone (ms); 0 = never.
    pub last_dump_visit_ms: u64,
    /// Last completed dump: first frag → PostDumpDone (ms); 0 = n/a.
    pub last_dump_bulk_ms: u64,
    /// Last completed dump: transport-end → PostDumpDone (ms); 0 = never.
    pub last_dump_post_ms: u64,
    /// Wall time (`now_ms`) of last PostDumpDone sequence.
    pub last_dump_done_ms: u64,
    /// Last completed dump: first `PeerJoin` → `PeerJoinAck` (ms); 0 = n/a.
    pub last_dump_queue_ms: u64,
    /// Last completed dump: first bulk frag → `TRANSPORT_END` (ms); 0 = n/a.
    pub last_dump_bulk_xfer_ms: u64,
    /// Last sane RTT from post-dump TimeSyncPing→Pong (ms); 0 = unknown.
    pub time_sync_rtt_ms: u16,
    /// Last one-way delay estimate (`rtt/2`); 0 = unknown.
    pub time_sync_one_way_ms: u16,
    /// Wall `now_ms` when last RTT-corrected TimeSet was sent (0 = never).
    pub last_rtt_time_set_ms: u64,
}

/// `SnifferSlot::adoption_state` wire values (dashboard / `/api/stats`).
pub const ADOPTION_EMPTY: u8 = 0;
pub const ADOPTION_PENDING: u8 = 1;
pub const ADOPTION_ADOPTED: u8 = 2;
pub const ADOPTION_REMOVED: u8 = 3;

const EMPTY_SNIFFER_SLOT: SnifferSlot = SnifferSlot {
    sniffer_id: 0,
    mac: [0; 6],
    last_seen_ms: 0,
    wifi_frag_batches: 0,
    ble_frag_batches: 0,
    ap_entries: 0,
    ble_entries: 0,
    probe_frag_batches: 0,
    probe_entries: 0,
    state: 0,
    ack_cmd_seq: 0,
    role: 0,
    ble_capable: 0,
    five_ghz_capable: 0, // NEW
    wifi_reasm_dropped: 0,
    ble_reasm_dropped: 0,
    wifi_rows_enqueued: 0,
    ble_rows_enqueued: 0,
    free_heap_bytes: 0,
    total_heap_bytes: 0,
    cpu_load_pct: 0,
    dfs_capable: 0,
    assigned_band: 0,
    pinned_channel: 0,
    assigned_chan_count: 0,
    assigned_channels: [0u8; wardriver_proto::CHANNELS_MAX],
    wifi_bulk_lost_cycles: 0,
    ble_bulk_lost_cycles: 0,
    sniffer_boot_id: 0,
    reboot_count: 0,
    fleet_slot: 0,
    adoption_state: ADOPTION_EMPTY,
    last_dump_visit_ms: 0,
    last_dump_bulk_ms: 0,
    last_dump_post_ms: 0,
    last_dump_done_ms: 0,
    last_dump_queue_ms: 0,
    last_dump_bulk_xfer_ms: 0,
    time_sync_rtt_ms: 0,
    time_sync_one_way_ms: 0,
    last_rtt_time_set_ms: 0,
};

/// Rows in the bounded sniffer table (dashboard + `snapshot_sniffers`). Tied to
/// [`crate::fleet::FLEET_MAX_SNIFFERS`]; ESP-NOW encrypt peers and SoftAP STAs share the
/// ESP32-C5 17-key CCMP pool (16 peers + 1 operator SoftAP slot).
pub const SNIFFER_TABLE_SLOTS: usize = crate::fleet::FLEET_MAX_SNIFFERS;

// Bounded list of sniffers for dashboard. v1: up to `SNIFFER_TABLE_SLOTS`.
static SNIFFERS: Mutex<[SnifferSlot; SNIFFER_TABLE_SLOTS]> =
    Mutex::new([EMPTY_SNIFFER_SLOT; SNIFFER_TABLE_SLOTS]);

/// Sniffer reboot transitions observed on control (see [`note_sniffer_boot_id`]).
pub static SNIFFER_REBOOTS: AtomicU32 = AtomicU32::new(0);

/// Incremented when [`sniffer_seen_from_ack`] cannot find a free slot in the bounded table.
/// At cap this catches MAC churn (NVS wipes producing new sniffer_ids) silently
/// exceeding the cap; at N>7 it is the single visible signal that the cap was hit.
pub static SNIFFERS_OVERFLOW: AtomicU32 = AtomicU32::new(0);

/// Record measured `set_channel` time for internal hop-average (control-silence timeout math).
#[cfg(feature = "sniffer")]
pub fn sniffer_record_hop_timing(set_channel_us: u32) {
    if let Ok(mut agg) = SNIFFER_HOP_AGG.lock() {
        agg.hop_count = agg.hop_count.saturating_add(1);
        agg.set_channel_us_sum = agg.set_channel_us_sum.saturating_add(set_channel_us as u64);
    }
}

/// Populate the per-sniffer plan view used by the dashboard. Called by the control planner
/// after computing [`wardriver_control::channel_plan::Effective`].
pub fn set_sniffer_plan_assignment(
    sniffer_id: u32,
    assigned_band: u8,
    pinned_channel: u8,
    chan_count: u8,
    channels: &[u8; wardriver_proto::CHANNELS_MAX],
) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.assigned_band = assigned_band.min(2);
                s.pinned_channel = pinned_channel;
                s.assigned_chan_count = chan_count.min(wardriver_proto::CHANNELS_MAX as u8);
                s.assigned_channels = *channels;
                return;
            }
        }
    }
}

/// Clear the per-sniffer plan view (e.g. when the sniffer drops out of the active window).
pub fn clear_sniffer_plan_assignment(sniffer_id: u32) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.assigned_band = 0;
                s.pinned_channel = 0;
                s.assigned_chan_count = 0;
                s.assigned_channels = [0u8; wardriver_proto::CHANNELS_MAX];
                return;
            }
        }
    }
}

/// Record `boot_id` on the sniffer slot. Returns `true` when `boot_id` changed after a prior
/// non-zero sighting (reboot); increments [`SnifferSlot::reboot_count`] and [`SNIFFER_REBOOTS`].
pub fn note_sniffer_boot_id(sniffer_id: u32, boot_id: u32) -> bool {
    if sniffer_id == 0 {
        return false;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                let prev = s.sniffer_boot_id;
                s.sniffer_boot_id = boot_id;
                if prev != 0 && prev != boot_id {
                    s.reboot_count = s.reboot_count.saturating_add(1);
                    s.wifi_bulk_lost_cycles = 0;
                    s.ble_bulk_lost_cycles = 0;
                    SNIFFER_REBOOTS.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                return false;
            }
        }
    }
    false
}

fn sniffer_seen_minimal_inner(sniffer_id: u32, mac: &[u8; 6]) {
    if sniffer_id == 0 {
        return;
    }
    let now = now_ms();
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                // Existing row: refresh liveness only; fleet_slot stays bound to table index.
                s.last_seen_ms = now;
                if s.mac == [0u8; 6] {
                    s.mac = *mac;
                }
                return;
            }
        }
        for (i, s) in slots.iter_mut().enumerate() {
            if s.sniffer_id == 0 {
                s.sniffer_id = sniffer_id;
                s.mac = *mac;
                s.last_seen_ms = now;
                // Table index + 1 → stable fleet slot for this control boot session.
                s.fleet_slot = (i + 1) as u8;
                // state=0 → Idle until a full SnifferAck refresh arrives.
                s.state = 0;
                s.adoption_state = ADOPTION_EMPTY;
                return;
            }
        }
        SNIFFERS_OVERFLOW.fetch_add(1, Ordering::Relaxed);
    }
}

/// Hello-path registration: marks adoption Pending when not roster-adopted.
pub fn sniffer_seen_minimal(sniffer_id: u32, mac: &[u8; 6], roster_adopted: bool) {
    sniffer_seen_minimal_inner(sniffer_id, mac);
    apply_adoption_on_seen(sniffer_id, roster_adopted);
}

/// Non-hello liveness refresh (fragments): does not change adoption state.
pub fn sniffer_seen_liveness(sniffer_id: u32, mac: &[u8; 6]) {
    sniffer_seen_minimal_inner(sniffer_id, mac);
}

fn apply_adoption_on_seen(sniffer_id: u32, roster_adopted: bool) {
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                if roster_adopted {
                    if s.adoption_state != ADOPTION_ADOPTED {
                        s.adoption_state = ADOPTION_ADOPTED;
                    }
                } else if s.adoption_state == ADOPTION_EMPTY {
                    s.adoption_state = ADOPTION_PENDING;
                }
                return;
            }
        }
    }
}

pub fn set_adoption_adopted(sniffer_id: u32, fleet_slot: u8) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.adoption_state = ADOPTION_ADOPTED;
                if fleet_slot != 0 {
                    s.fleet_slot = fleet_slot;
                }
                return;
            }
        }
    }
}

pub fn set_adoption_pending(sniffer_id: u32, fleet_slot: u8) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                if s.adoption_state != ADOPTION_ADOPTED {
                    s.adoption_state = ADOPTION_PENDING;
                }
                if fleet_slot != 0 {
                    s.fleet_slot = fleet_slot;
                }
                return;
            }
        }
    }
}

pub fn set_adoption_removed(sniffer_id: u32) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.adoption_state = ADOPTION_REMOVED;
                return;
            }
        }
    }
}

pub fn sniffer_seen_from_ack(sniffer_id: u32, mac: &[u8; 6], ack: &wardriver_proto::SnifferAckV1) {
    let now = now_ms();
    if let Ok(mut slots) = SNIFFERS.lock() {
        let mut idx = None;
        for (i, s) in slots.iter().enumerate() {
            if s.sniffer_id == sniffer_id && sniffer_id != 0 {
                idx = Some(i);
                break;
            }
        }
        if idx.is_none() {
            for (i, s) in slots.iter().enumerate() {
                if s.sniffer_id == 0 {
                    idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = idx {
            slots[i].sniffer_id = sniffer_id;
            slots[i].mac = *mac;
            slots[i].last_seen_ms = now;
            slots[i].state = ack.state as u8;
            slots[i].ack_cmd_seq = ack.ack_cmd_seq;
            slots[i].role = (ack.role as u8).min(1);
            slots[i].ble_capable = ack.ble_capable.min(1);
            slots[i].five_ghz_capable = ack.five_ghz_capable.min(1);
            slots[i].free_heap_bytes = ack.free_heap_bytes;
            slots[i].total_heap_bytes = ack.total_heap_bytes;
            slots[i].cpu_load_pct = ack.cpu_load_pct.min(100);
            slots[i].dfs_capable = ack.five_ghz_capable.min(1);
            // SnifferAck carries one cumulative bulk-lost counter; attribute by current role.
            // After SetRole the value still reflects evictions since sniffer boot (wire unchanged).
            match ack.role {
                wardriver_proto::SnifferRole::Ble => {
                    slots[i].ble_bulk_lost_cycles = ack.bulk_lost_cycles;
                }
                wardriver_proto::SnifferRole::Wifi => {
                    slots[i].wifi_bulk_lost_cycles = ack.bulk_lost_cycles;
                }
            }
            slots[i].sniffer_boot_id = ack.sniffer_boot_id;
        } else if sniffer_id != 0 {
            SNIFFERS_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn sniffer_wifi_rows_enqueued(sniffer_id: u32, n: usize) {
    if n == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.wifi_rows_enqueued = s.wifi_rows_enqueued.saturating_add(n as u64);
                return;
            }
        }
    }
}

pub fn sniffer_ble_rows_enqueued(sniffer_id: u32, n: usize) {
    if n == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.ble_rows_enqueued = s.ble_rows_enqueued.saturating_add(n as u64);
                return;
            }
        }
    }
}

pub fn sniffers_overflow() -> u32 {
    SNIFFERS_OVERFLOW.load(Ordering::Relaxed)
}

pub fn sniffer_frame(sniffer_id: u32, ap_entries: usize) {
    let now = now_ms();
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.last_seen_ms = now;
                s.wifi_frag_batches += 1;
                s.ap_entries += ap_entries as u64;
                return;
            }
        }
    }
}

pub fn sniffer_ble_frame(sniffer_id: u32, ble_entries: usize) {
    let now = now_ms();
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.last_seen_ms = now;
                s.ble_frag_batches += 1;
                s.ble_entries += ble_entries as u64;
                return;
            }
        }
    }
}

pub fn sniffer_probe_frame(sniffer_id: u32, probe_entries: usize) {
    let now = now_ms();
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.last_seen_ms = now;
                s.probe_frag_batches += 1;
                s.probe_entries += probe_entries as u64;
                return;
            }
        }
    }
}

/// Record per-sniffer dump duration telemetry when control sends `PostDumpDone`.
pub fn record_dump_complete(
    sniffer_id: u32,
    visit_ms: u64,
    bulk_ms: u64,
    post_ms: u64,
    done_ms: u64,
    queue_ms: u64,
    bulk_xfer_ms: u64,
) {
    if sniffer_id == 0 {
        return;
    }
    use std::sync::atomic::Ordering;
    crate::counters::WOLFPACK_DUMP_SAMPLES.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut sum) = crate::counters::WOLFPACK_QUEUE_MS_SUM.lock() {
        *sum = sum.saturating_add(queue_ms);
    }
    if queue_ms > 0 {
        crate::counters::WOLFPACK_QUEUE_MS_MAX.fetch_max(queue_ms as u32, Ordering::Relaxed);
    }
    if let Ok(mut sum) = crate::counters::WOLFPACK_BULK_XFER_MS_SUM.lock() {
        *sum = sum.saturating_add(bulk_xfer_ms);
    }
    if bulk_xfer_ms > 0 {
        crate::counters::WOLFPACK_BULK_XFER_MS_MAX
            .fetch_max(bulk_xfer_ms as u32, Ordering::Relaxed);
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.last_dump_visit_ms = visit_ms;
                s.last_dump_bulk_ms = bulk_ms;
                s.last_dump_post_ms = post_ms;
                s.last_dump_done_ms = done_ms;
                s.last_dump_queue_ms = queue_ms;
                s.last_dump_bulk_xfer_ms = bulk_xfer_ms;
                return;
            }
        }
    }
}

/// Lookup `fleet_slot` for a sniffer by id; 0 if unknown or unassigned.
pub fn fleet_slot_for_sniffer(sniffer_id: u32) -> u8 {
    if sniffer_id == 0 {
        return 0;
    }
    if let Ok(slots) = SNIFFERS.lock() {
        for s in slots.iter() {
            if s.sniffer_id == sniffer_id {
                return s.fleet_slot;
            }
        }
    }
    0
}

/// Lookup dashboard sniffer id for an ESP-NOW peer MAC (no full-table stack copy).
#[must_use]
pub fn sniffer_id_for_mac(mac: &[u8; 6]) -> Option<u32> {
    let Ok(slots) = SNIFFERS.lock() else {
        return None;
    };
    for s in slots.iter() {
        if s.sniffer_id != 0 && s.mac == *mac {
            return Some(s.sniffer_id);
        }
    }
    None
}

/// Roster `last_seen_ms` for a sniffer id (`None` when unknown or id 0).
#[must_use]
pub fn sniffer_last_seen_ms(sniffer_id: u32) -> Option<u64> {
    if sniffer_id == 0 {
        return None;
    }
    let Ok(slots) = SNIFFERS.lock() else {
        return None;
    };
    for s in slots.iter() {
        if s.sniffer_id == sniffer_id {
            return Some(s.last_seen_ms);
        }
    }
    None
}

pub fn sniffer_last_rtt_time_set_ms(sniffer_id: u32) -> u64 {
    if sniffer_id == 0 {
        return 0;
    }
    if let Ok(slots) = SNIFFERS.lock() {
        for s in slots.iter() {
            if s.sniffer_id == sniffer_id {
                return s.last_rtt_time_set_ms;
            }
        }
    }
    0
}

pub fn set_sniffer_last_rtt_time_set_ms(sniffer_id: u32, ms: u64) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.last_rtt_time_set_ms = ms;
                return;
            }
        }
    }
}

/// Clear per-sniffer RTT interval markers when control wall clock first becomes trusted.
pub fn clear_all_sniffer_last_rtt_time_set_ms() {
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            s.last_rtt_time_set_ms = 0;
        }
    }
}

pub fn record_sniffer_time_sync_rtt(sniffer_id: u32, rtt_ms: u16, one_way_ms: u16) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut slots) = SNIFFERS.lock() {
        for s in slots.iter_mut() {
            if s.sniffer_id == sniffer_id {
                s.time_sync_rtt_ms = rtt_ms;
                s.time_sync_one_way_ms = one_way_ms;
                return;
            }
        }
    }
}

pub fn snapshot_sniffers() -> [SnifferSlot; SNIFFER_TABLE_SLOTS] {
    if let Ok(slots) = SNIFFERS.lock() {
        *slots
    } else {
        [SnifferSlot::default(); SNIFFER_TABLE_SLOTS]
    }
}

/// Iterate live sniffer rows without copying the full table (safe on small stacks).
pub fn for_each_sniffer_slot(mut f: impl FnMut(&SnifferSlot)) {
    let Ok(slots) = SNIFFERS.lock() else {
        return;
    };
    for s in slots.iter() {
        if s.sniffer_id != 0 {
            f(s);
        }
    }
}

/// Registered sniffers with WiFi role (`role == 0`) for channel-plan math (no active-window filter).
pub fn fill_wifi_role_sniffer_ids(out: &mut Vec<u32>) {
    out.clear();
    for_each_sniffer_slot(|s| {
        if s.role == 0 {
            out.push(s.sniffer_id);
        }
    });
    out.sort_unstable();
    out.dedup();
}

/// Wire role (`0` WiFi, `1` BLE) for a dashboard sniffer id.
#[must_use]
pub fn role_for_sniffer_id(sniffer_id: u32) -> Option<u8> {
    if sniffer_id == 0 {
        return None;
    }
    let Ok(slots) = SNIFFERS.lock() else {
        return None;
    };
    slots
        .iter()
        .find(|s| s.sniffer_id == sniffer_id)
        .map(|s| s.role.min(1))
}

const DEFERRED_WELCOME_MIN_INTERVAL_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug)]
pub struct PendingDeferredWelcome {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub fleet_slot: u8,
    /// Adopt POST and similar: bypass rate limit at drain.
    pub force: bool,
}

static PENDING_DEFERRED_WELCOME: Mutex<Vec<PendingDeferredWelcome>> = Mutex::new(Vec::new());
static LAST_DEFERRED_WELCOME_MS: OnceLock<Mutex<HashMap<u32, u64>>> = OnceLock::new();

fn last_deferred_welcome_ms() -> &'static Mutex<HashMap<u32, u64>> {
    LAST_DEFERRED_WELCOME_MS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn welcome_rate_allows(sniffer_id: u32, now_ms: u64) -> bool {
    let Ok(last) = last_deferred_welcome_ms().lock() else {
        return true;
    };
    last.get(&sniffer_id)
        .map(|t| now_ms.saturating_sub(*t) >= DEFERRED_WELCOME_MIN_INTERVAL_MS)
        .unwrap_or(true)
}

/// Queue welcome ESP-NOW work for the control main loop (not ingest / httpd stacks).
pub fn enqueue_deferred_welcome(sniffer_id: u32, mac: [u8; 6], fleet_slot: u8, force: bool) {
    if let Ok(mut q) = PENDING_DEFERRED_WELCOME.lock() {
        if let Some(pos) = q.iter().position(|j| j.sniffer_id == sniffer_id) {
            let existing = &mut q[pos];
            existing.mac = mac;
            existing.fleet_slot = fleet_slot;
            existing.force = existing.force || force;
        } else {
            q.push(PendingDeferredWelcome {
                sniffer_id,
                mac,
                fleet_slot,
                force,
            });
        }
    }
}

/// Adopt POST: immediate welcome at next main-loop drain.
pub fn enqueue_adopt_welcome(sniffer_id: u32, mac: [u8; 6], fleet_slot: u8) {
    enqueue_deferred_welcome(sniffer_id, mac, fleet_slot, true);
}

#[must_use]
pub fn take_pending_deferred_welcomes(now_ms: u64) -> Vec<PendingDeferredWelcome> {
    let jobs = PENDING_DEFERRED_WELCOME
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    jobs.into_iter()
        .filter(|j| j.force || welcome_rate_allows(j.sniffer_id, now_ms))
        .collect()
}

pub fn mark_deferred_welcome_sent(sniffer_id: u32, now_ms: u64) {
    if let Ok(mut last) = last_deferred_welcome_ms().lock() {
        last.insert(sniffer_id, now_ms);
    }
}

/// Lookup peer MAC without copying the full sniffer table (safe on httpd PSRAM stack).
#[must_use]
pub fn mac_for_sniffer_id(sniffer_id: u32) -> Option<[u8; 6]> {
    if sniffer_id == 0 {
        return None;
    }
    let Ok(slots) = SNIFFERS.lock() else {
        return None;
    };
    for s in slots.iter() {
        if s.sniffer_id == sniffer_id && s.mac != [0u8; 6] {
            return Some(s.mac);
        }
    }
    None
}
