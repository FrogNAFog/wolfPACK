use wardriver_core::ingest_limits::{
    self, mac_table_limits_from_combined_budget, MAC_TABLE_GEO_SLOTS_CAP,
};
use wardriver_stats as stats;

/// Combined MAC table + geo slot heap budgets per runtime tier (sums unchanged from prior split).
const MAC_GEO_BUDGET_LOW_BYTES: usize = 512 * 1024 + 128 * 1024;
const MAC_GEO_BUDGET_MID_BYTES: usize = 1024 * 1024 + 256 * 1024;
const MAC_GEO_BUDGET_HIGH_BYTES: usize = 2 * 1024 * 1024 + 512 * 1024;

const CONTROL_PSRAM_DEDUP_MAX: usize = 16_384;
const CONTROL_PSRAM_REASM_MAX: usize = 512;
const CONTROL_PSRAM_SD_QUEUE: usize = 128;
const CONTROL_PSRAM_SLICE_FRAMES: usize = 512;

/// Per-lane deferred wardrive bulk budgets on sniffers (SPIRAM vec prealloc each).
pub const OUTBOUND_BULK_WIFI_BUDGET_BYTES: usize = 64 * 1024;
pub const OUTBOUND_BULK_PROBE_BUDGET_BYTES: usize = 64 * 1024;
pub const OUTBOUND_BULK_BLE_BUDGET_BYTES: usize = 64 * 1024;

/// Sum of outbound bulk lane budgets (boot logging).
pub const OUTBOUND_BULK_BUDGET_BYTES: usize = OUTBOUND_BULK_WIFI_BUDGET_BYTES
    + OUTBOUND_BULK_PROBE_BUDGET_BYTES
    + OUTBOUND_BULK_BLE_BUDGET_BYTES;

/// Promisc RX ring prealloc for fleet sniffers (192 KiB).
const PROMISC_QUEUE_BUDGET_BYTES: usize = 192 * 1024;
/// `wifi_promisc::PromiscRecord` layout (`PROMISC_MAX_COPY` + RX metadata).
const EST_BYTES_PER_PROMISC_SLOT: usize = 1024 + 8;

fn promisc_capacity_from_budget(budget_bytes: usize) -> usize {
    (budget_bytes / EST_BYTES_PER_PROMISC_SLOT).clamp(16, 1024)
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeMemoryPolicy {
    /// Sniffer ESP-NOW RX depth; control uses [`crate::espnow::ESPNOW_RX_QUEUE_CAPACITY_CONTROL`].
    #[cfg_attr(not(feature = "sniffer"), allow(dead_code))]
    pub espnow_rx_queue_capacity: usize,
    /// Used by sniffer firmware only; control builds keep the value for a shared policy struct.
    #[cfg_attr(not(feature = "sniffer"), allow(dead_code))]
    pub promisc_queue_capacity: usize,
    pub sd_writer_queue_capacity: usize,
    pub dedup_max_entries: usize,
    pub reasm_max_entries: usize,
    pub mac_table_max_entries: usize,
    pub mac_table_geo_slots_max: usize,
    /// Phase 1 compat: mirrors [`Self::mac_table_max_entries`] until ingest uses one unified table.
    pub row_dedup_max_wifi_macs: usize,
    /// Phase 1 compat: mirrors [`Self::mac_table_max_entries`] until ingest uses one unified table.
    pub row_dedup_max_ble_macs: usize,
    pub row_dedup_heap_trim_bytes: u32,
    pub control_loop_slice_frames: usize,
    pub maintenance_items_per_tick: usize,
}

fn detect_profile() -> RuntimeMemoryPolicy {
    let free_total = stats::free_heap_bytes();
    let free_internal = stats::free_internal_heap_bytes();
    let free_spiram = stats::free_spiram_heap_bytes();
    let has_psram = free_spiram > 0;

    let combined_budget = if has_psram && free_total > 1_500_000 {
        MAC_GEO_BUDGET_HIGH_BYTES
    } else if free_total > 300_000 {
        MAC_GEO_BUDGET_MID_BYTES
    } else {
        MAC_GEO_BUDGET_LOW_BYTES
    };

    #[cfg(feature = "control")]
    let geo_slots_cap = MAC_TABLE_GEO_SLOTS_CAP;
    #[cfg(not(feature = "control"))]
    let geo_slots_cap = 0;

    let (mac_table_max_entries, mac_table_geo_slots_max) =
        mac_table_limits_from_combined_budget(combined_budget, geo_slots_cap);

    let (espnow_rx, sdq, dedup, reasm, ctl_drain) = if has_psram && free_total > 1_500_000 {
        (192, 1024, 8192, 320, 512)
    } else if free_total > 300_000 {
        (128, 512, 4096, 128, 384)
    } else {
        (96, 384, 3072, 96, 256)
    };

    let promisc_rx = promisc_capacity_from_budget(PROMISC_QUEUE_BUDGET_BYTES);

    let row_trim = free_internal.saturating_div(4).clamp(24_000, 96_000);
    RuntimeMemoryPolicy {
        espnow_rx_queue_capacity: espnow_rx,
        promisc_queue_capacity: promisc_rx,
        sd_writer_queue_capacity: sdq,
        dedup_max_entries: dedup,
        reasm_max_entries: reasm,
        mac_table_max_entries,
        mac_table_geo_slots_max,
        row_dedup_max_wifi_macs: mac_table_max_entries,
        row_dedup_max_ble_macs: mac_table_max_entries,
        row_dedup_heap_trim_bytes: row_trim,
        control_loop_slice_frames: ctl_drain,
        maintenance_items_per_tick: 64,
    }
}

pub fn load_runtime_policy() -> RuntimeMemoryPolicy {
    detect_profile()
}

/// Control-node policy: generous PSRAM caps when present; emergency shrink on low internal heap.
#[must_use]
pub fn load_control_runtime_policy() -> RuntimeMemoryPolicy {
    let mut p = load_runtime_policy();
    if stats::free_spiram_heap_bytes() > 0 {
        let (mac_max, geo_max) = ingest_limits::control_mac_table_limits();
        p.mac_table_max_entries = mac_max;
        p.mac_table_geo_slots_max = geo_max;
        p.row_dedup_max_wifi_macs = mac_max;
        p.row_dedup_max_ble_macs = mac_max;
        p.dedup_max_entries = CONTROL_PSRAM_DEDUP_MAX;
        p.reasm_max_entries = CONTROL_PSRAM_REASM_MAX;
        p.sd_writer_queue_capacity = CONTROL_PSRAM_SD_QUEUE;
        p.control_loop_slice_frames = CONTROL_PSRAM_SLICE_FRAMES;
    }
    p.apply_control_node_limits();
    p
}

impl RuntimeMemoryPolicy {
    /// Tighten SD queue caps; scale down further when internal heap is low.
    /// ESP-NOW RX depth on control is fixed in [`crate::espnow::ESPNOW_RX_QUEUE_CAPACITY_CONTROL`].
    pub fn apply_control_node_limits(&mut self) {
        self.sd_writer_queue_capacity = self.sd_writer_queue_capacity.min(128);
        self.control_loop_slice_frames = self.control_loop_slice_frames.min(512);

        let internal_free = stats::free_internal_heap_bytes();
        if internal_free < 120_000 {
            self.sd_writer_queue_capacity = self.sd_writer_queue_capacity.min(64);
            self.reasm_max_entries = self.reasm_max_entries.min(64);
            self.dedup_max_entries = self.dedup_max_entries.min(1024);
            self.control_loop_slice_frames = self.control_loop_slice_frames.min(64);
        }
    }
}
