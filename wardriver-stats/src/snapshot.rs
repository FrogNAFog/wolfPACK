//! Tier-A runtime snapshot types for `/api/stats` (schema rev 2).

use serde::Serialize;

pub const STATS_SCHEMA_REV: u32 = 2;

#[derive(Serialize)]
pub struct SnifferSlotSnapshot {
    pub sniffer_id: u32,
    pub mac: String,
    pub last_seen_ms: u64,
    /// Legacy sum: `wifi_frag_batches + ble_frag_batches`.
    pub frames: u64,
    pub wifi_frag_batches: u64,
    pub ble_frag_batches: u64,
    pub ap_entries: u64,
    pub ble_entries: u64,
    pub state: u8,
    pub ack_cmd_seq: u32,
    pub role: u8,
    pub ble_capable: u8,
    pub five_ghz_capable: u8,
    /// PROTO cleanup candidate: mirrors `five_ghz_capable` until next PROTO bump.
    pub dfs_capable: u8,
    /// Legacy sum: `wifi_reasm_dropped + ble_reasm_dropped`.
    pub reasm_dropped: u64,
    pub wifi_reasm_dropped: u64,
    pub ble_reasm_dropped: u64,
    /// Legacy sum: `wifi_rows_enqueued + ble_rows_enqueued`.
    pub rows_enqueued: u64,
    pub wifi_rows_enqueued: u64,
    pub ble_rows_enqueued: u64,
    pub mem_heap_free: u32,
    pub mem_heap_total: u32,
    pub mem_heap_avail_pct: Option<u8>,
    pub cpu_load_pct: u8,
    pub assigned_band: u8,
    pub pinned_channel: u8,
    pub assigned_channels: Vec<u8>,
    /// Legacy sum: `wifi_bulk_lost_cycles + ble_bulk_lost_cycles`.
    pub bulk_lost_cycles: u32,
    pub wifi_bulk_lost_cycles: u32,
    pub ble_bulk_lost_cycles: u32,
    pub sniffer_boot_id: u32,
    pub reboot_count: u32,
    pub fleet_slot: u8,
    pub adoption_state: u8,
    pub last_dump_visit_ms: u64,
    pub last_dump_bulk_ms: u64,
    pub last_dump_post_ms: u64,
    pub last_dump_done_ms: u64,
    pub last_dump_queue_ms: u64,
    pub last_dump_bulk_xfer_ms: u64,
    pub time_sync_rtt_ms: u16,
    pub time_sync_one_way_ms: u16,
    pub last_rtt_time_set_ms: u64,
}

/// Last detection alert snapshot for nested `/api/stats` JSON (`detection_last` object).
#[derive(Serialize, Clone)]
pub struct DetectionLastSnapshot {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: String,
    pub wildcard_count: u16,
    pub channel: u8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub oui_matched: u8,
    pub signal_kind: u8,
    pub detection_method: u8,
    pub ble_company_id: u16,
}

/// Last mobile-surveillance alert snapshot for nested `/api/stats` JSON (PACK Axon OUI).
#[derive(Serialize, Clone)]
pub struct MobileSurveillanceLastSnapshot {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: String,
    pub signal_kind: u8,
    pub detection_method: u8,
}

/// Last SSID watch beacon hit for nested `/api/stats` JSON.
#[derive(Serialize, Clone)]
pub struct SsidWatchLastSnapshot {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub bssid: String,
    pub channel: u8,
    pub rssi: i8,
    pub ssid: String,
}

/// Latest published fix fields for GPS snapshot evaluation (control passes `gps::Fix` fields).
#[derive(Clone, Copy, Debug)]
pub struct GpsFixInput {
    pub fix_quality: u8,
    pub hdop: Option<f32>,
    pub satellites: Option<u8>,
}

/// GPS gate and freshness snapshot for nested `/api/stats` JSON (`gps` object).
#[derive(Serialize, Clone)]
pub struct GpsSnapshot {
    pub fix_ok: u8,
    pub hdop: Option<f32>,
    pub satellites: Option<u8>,
    pub uart_fresh: u8,
    pub rmc_fresh: u8,
    pub gga_fresh: u8,
    pub nav_fresh: u8,
    pub clock_stale: u8,
    pub data_fresh: u8,
    pub log_reason: String,
}

#[derive(Serialize)]
pub struct RuntimeSnapshotTierA {
    pub stats_schema_rev: u32,
    pub now_ms: u64,
    pub uptime_ms: u64,
    pub scan_allowed: u8,
    pub csv_enqueue_allowed: u8,
    pub sniffers: Vec<SnifferSlotSnapshot>,
    pub detection_last: DetectionLastSnapshot,
    pub mobile_surveillance_last: MobileSurveillanceLastSnapshot,
    pub ssid_watch_last: SsidWatchLastSnapshot,
    pub gps: GpsSnapshot,
}
