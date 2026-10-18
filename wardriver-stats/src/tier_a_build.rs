//! Tier-A `/api/stats` snapshot builders (pure; no atomics or control types).

use crate::snapshot::{
    DetectionLastSnapshot, GpsFixInput, GpsSnapshot, MobileSurveillanceLastSnapshot,
    RuntimeSnapshotTierA, SnifferSlotSnapshot, SsidWatchLastSnapshot, STATS_SCHEMA_REV,
};

const GPS_MIN_FIX_QUALITY: u8 = 1;
const GPS_MAX_HDOP: f32 = 5.0;

fn format_bssid_mac(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn heap_avail_pct_from(free: u32, total: u32) -> Option<u8> {
    if total == 0 {
        return None;
    }
    let free = free.min(total);
    Some(((free as u64 * 100) / total as u64).min(100) as u8)
}

fn evaluate_gps_fix(fix_quality: u8, hdop: Option<f32>) -> (u8, String, u8, Option<f32>) {
    if fix_quality < GPS_MIN_FIX_QUALITY {
        return (
            0,
            format!("fix_quality {fix_quality} < {GPS_MIN_FIX_QUALITY}"),
            fix_quality,
            hdop,
        );
    }
    match hdop {
        None => (0, "missing HDOP (need GGA)".to_string(), fix_quality, None),
        Some(h) if !h.is_finite() || h <= 0.0 => {
            (0, format!("invalid HDOP {h}"), fix_quality, Some(h))
        }
        Some(h) if h > GPS_MAX_HDOP => (
            0,
            format!("HDOP {h:.2} > {GPS_MAX_HDOP:.2}"),
            fix_quality,
            Some(h),
        ),
        Some(h) => (1, format!("OK (HDOP {h:.2})"), fix_quality, Some(h)),
    }
}

/// GPS freshness and auxiliary fields read from shared stats atomics before snapshot build.
#[derive(Clone, Copy, Debug)]
pub struct StatsGpsInputs {
    pub uart_fresh: u8,
    pub rmc_fresh: u8,
    pub gga_fresh: u8,
    pub nav_fresh: u8,
    pub clock_stale: u8,
    pub data_fresh: u8,
    pub rmc_age_ms: u64,
    pub gga_age_ms: u64,
    pub satellites_in_view: Option<u8>,
}

pub(crate) fn build_gps_snapshot(fix: Option<GpsFixInput>, stats: StatsGpsInputs) -> GpsSnapshot {
    let (fix_ok, fix_reason, _, hdop) = match fix {
        None => (0u8, "no fix yet".to_string(), 0u8, None),
        Some(f) => evaluate_gps_fix(f.fix_quality, f.hdop),
    };
    let log_reason = if fix_ok != 0 && stats.nav_fresh != 0 && stats.clock_stale == 0 {
        fix_reason.clone()
    } else if fix_ok == 0 {
        fix_reason
    } else if stats.clock_stale != 0 {
        format!("clock stale (rmc_age_ms={})", stats.rmc_age_ms)
    } else if stats.nav_fresh == 0 {
        format!(
            "nav stale (rmc_age_ms={} gga_age_ms={})",
            stats.rmc_age_ms, stats.gga_age_ms
        )
    } else {
        fix_reason
    };
    let satellites = fix.and_then(|f| f.satellites).or(stats.satellites_in_view);
    GpsSnapshot {
        fix_ok,
        hdop,
        satellites,
        uart_fresh: stats.uart_fresh,
        rmc_fresh: stats.rmc_fresh,
        gga_fresh: stats.gga_fresh,
        nav_fresh: stats.nav_fresh,
        clock_stale: stats.clock_stale,
        data_fresh: stats.data_fresh,
        log_reason,
    }
}

/// Raw sniffer slot fields for tier-A snapshot assembly (MAC still `[u8; 6]`).
#[derive(Clone, Debug)]
pub struct SnifferSlotInput {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub last_seen_ms: u64,
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
    pub wifi_reasm_dropped: u64,
    pub ble_reasm_dropped: u64,
    pub wifi_rows_enqueued: u64,
    pub ble_rows_enqueued: u64,
    pub free_heap_bytes: u32,
    pub total_heap_bytes: u32,
    pub cpu_load_pct: u8,
    pub assigned_band: u8,
    pub pinned_channel: u8,
    pub assigned_channels: Vec<u8>,
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

#[derive(Clone, Copy, Debug)]
pub struct DetectionLastInput {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: [u8; 6],
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

#[derive(Clone, Copy, Debug)]
pub struct MobileSurveillanceLastInput {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub signal_kind: u8,
    pub detection_method: u8,
}

#[derive(Clone, Debug)]
pub struct SsidWatchLastInput {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub bssid: [u8; 6],
    pub channel: u8,
    pub rssi: i8,
    pub ssid: String,
}

pub(crate) fn build_runtime_snapshot_tier_a(
    now_ms: u64,
    boot_ms: u64,
    scan_allowed: u8,
    csv_enqueue_allowed: u8,
    sniffers: &[SnifferSlotInput],
    detection_last: DetectionLastInput,
    mobile_surveillance_last: MobileSurveillanceLastInput,
    ssid_watch_last: SsidWatchLastInput,
    gps_fix: Option<GpsFixInput>,
    gps_stats: StatsGpsInputs,
) -> RuntimeSnapshotTierA {
    let uptime_ms = now_ms.saturating_sub(boot_ms);
    let sniffers: Vec<SnifferSlotSnapshot> = sniffers
        .iter()
        .map(|s| SnifferSlotSnapshot {
            sniffer_id: s.sniffer_id,
            mac: format_bssid_mac(&s.mac),
            last_seen_ms: s.last_seen_ms,
            frames: s.wifi_frag_batches.saturating_add(s.ble_frag_batches),
            wifi_frag_batches: s.wifi_frag_batches,
            ble_frag_batches: s.ble_frag_batches,
            ap_entries: s.ap_entries,
            ble_entries: s.ble_entries,
            state: s.state,
            ack_cmd_seq: s.ack_cmd_seq,
            role: s.role,
            ble_capable: s.ble_capable,
            five_ghz_capable: s.five_ghz_capable,
            dfs_capable: s.dfs_capable,
            reasm_dropped: s.wifi_reasm_dropped.saturating_add(s.ble_reasm_dropped),
            wifi_reasm_dropped: s.wifi_reasm_dropped,
            ble_reasm_dropped: s.ble_reasm_dropped,
            rows_enqueued: s.wifi_rows_enqueued.saturating_add(s.ble_rows_enqueued),
            wifi_rows_enqueued: s.wifi_rows_enqueued,
            ble_rows_enqueued: s.ble_rows_enqueued,
            mem_heap_free: s.free_heap_bytes,
            mem_heap_total: s.total_heap_bytes,
            mem_heap_avail_pct: heap_avail_pct_from(s.free_heap_bytes, s.total_heap_bytes),
            cpu_load_pct: s.cpu_load_pct.min(100),
            assigned_band: s.assigned_band,
            pinned_channel: s.pinned_channel,
            assigned_channels: s.assigned_channels.clone(),
            bulk_lost_cycles: s
                .wifi_bulk_lost_cycles
                .saturating_add(s.ble_bulk_lost_cycles),
            wifi_bulk_lost_cycles: s.wifi_bulk_lost_cycles,
            ble_bulk_lost_cycles: s.ble_bulk_lost_cycles,
            sniffer_boot_id: s.sniffer_boot_id,
            reboot_count: s.reboot_count,
            fleet_slot: s.fleet_slot,
            adoption_state: s.adoption_state,
            last_dump_visit_ms: s.last_dump_visit_ms,
            last_dump_bulk_ms: s.last_dump_bulk_ms,
            last_dump_post_ms: s.last_dump_post_ms,
            last_dump_done_ms: s.last_dump_done_ms,
            last_dump_queue_ms: s.last_dump_queue_ms,
            last_dump_bulk_xfer_ms: s.last_dump_bulk_xfer_ms,
            time_sync_rtt_ms: s.time_sync_rtt_ms,
            time_sync_one_way_ms: s.time_sync_one_way_ms,
            last_rtt_time_set_ms: s.last_rtt_time_set_ms,
        })
        .collect();
    let detection_last = DetectionLastSnapshot {
        last_seen_ms: detection_last.last_seen_ms,
        sniffer_id: detection_last.sniffer_id,
        src: format_bssid_mac(&detection_last.src),
        wildcard_count: detection_last.wildcard_count,
        channel: detection_last.channel,
        rssi_min: detection_last.rssi_min,
        rssi_max: detection_last.rssi_max,
        rssi_avg_q8: detection_last.rssi_avg_q8,
        oui_matched: detection_last.oui_matched,
        signal_kind: detection_last.signal_kind,
        detection_method: detection_last.detection_method,
        ble_company_id: detection_last.ble_company_id,
    };
    let mobile_surveillance_last = MobileSurveillanceLastSnapshot {
        last_seen_ms: mobile_surveillance_last.last_seen_ms,
        sniffer_id: mobile_surveillance_last.sniffer_id,
        src: format_bssid_mac(&mobile_surveillance_last.src),
        signal_kind: mobile_surveillance_last.signal_kind,
        detection_method: mobile_surveillance_last.detection_method,
    };
    let ssid_watch_last = SsidWatchLastSnapshot {
        last_seen_ms: ssid_watch_last.last_seen_ms,
        sniffer_id: ssid_watch_last.sniffer_id,
        bssid: format_bssid_mac(&ssid_watch_last.bssid),
        channel: ssid_watch_last.channel,
        rssi: ssid_watch_last.rssi,
        ssid: ssid_watch_last.ssid,
    };
    let gps = build_gps_snapshot(gps_fix, gps_stats);
    RuntimeSnapshotTierA {
        stats_schema_rev: STATS_SCHEMA_REV,
        now_ms,
        uptime_ms,
        scan_allowed,
        csv_enqueue_allowed,
        sniffers,
        detection_last,
        mobile_surveillance_last,
        ssid_watch_last,
        gps,
    }
}
