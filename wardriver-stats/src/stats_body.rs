//! Single source of truth for the `/api/stats` JSON body.
//!
//! The body is one serde-driven `ApiStatsBody` serialized with `serde_json::to_string`,
//! replacing the hand-built `write!` fragments that previously lived in
//! `wardriver-control/src/web/stats_json.rs`. Field order in [`ApiStatsBody`] is the
//! exact wire order of the emitted JSON; the fixed-point float fields are emitted
//! verbatim via [`serde_json::value::RawValue`] so their formatting (and the no-
//! `core::fmt`-float guarantee) is unchanged.
//!
//! The four nested objects (`sniffers`, `detection_last`, `mobile_surveillance_last`,
//! `ssid_watch_last`) reuse the existing serde types from [`crate::snapshot`], and the
//! `diagnostics` object is injected as a raw JSON fragment built by the control side.

use serde::Serialize;
use serde_json::value::RawValue;

use crate::fleet;
use crate::snapshot::{
    DetectionLastSnapshot, GpsFixInput, GpsSnapshot, MobileSurveillanceLastSnapshot,
    SsidWatchLastSnapshot,
};

/// `RawValue` helper: store a preformatted JSON fragment (number, `null`, object, …).
pub(crate) type Raw = Box<RawValue>;

fn raw(s: &str) -> Raw {
    RawValue::from_string(s.to_string()).expect("stats body fragment is valid JSON")
}

/// Raw JSON number fragments for the fixed-point float fields (no `core::fmt` float paths).
fn gps_hdop_raw(hdop: Option<f32>) -> Raw {
    match hdop {
        None => raw("null"),
        Some(h) => {
            let mut s = String::new();
            crate::json_fixed::push_decimal_2_from_x100(&mut s, crate::json_fixed::hdop_to_x100(h));
            raw(&s)
        }
    }
}

fn decimal_1_raw(x10: i32) -> Raw {
    let mut s = String::new();
    crate::json_fixed::push_decimal_1_from_x10(&mut s, x10);
    raw(&s)
}

fn degrees_7_raw(microdeg: i64) -> Raw {
    let mut s = String::new();
    crate::json_fixed::push_degrees_7_from_microdeg(&mut s, microdeg);
    raw(&s)
}

fn null_raw() -> Raw {
    raw("null")
}

/// The `/api/stats` JSON body. Field order == wire order.
///
/// `firmware_version` / `firmware_role` / `board_profile` / `esp_idf_version` /
/// `fleet_phase_name` / `upload_phase` are `String` so the control side can inject the
/// build-time / domain values (they are the only non-numeric, non-`Raw` string fields).
#[derive(Serialize)]
pub struct ApiStatsBody {
    pub forms_rev: u32,
    pub stats_schema_rev: u32,
    pub firmware_version: String,
    pub firmware_role: String,
    pub board_profile: String,
    pub proto_version: u32,
    pub esp_idf_version: String,
    pub now_ms: u64,
    pub uptime_ms: u64,
    pub control_mem_heap_avail_pct: u8,
    pub control_mem_heap_free_bytes: u32,
    pub control_mem_heap_total_bytes: u32,
    pub control_cpu_load_pct: u8,
    pub reset_reason: String,
    pub reset_reason_raw: u32,
    pub channel_plan_revision: u32,
    pub channel_plan_sd_load_fail: u32,
    pub channel_plan_sd_save_fail: u32,
    pub gps_time_valid: u8,
    pub wall_clock_trusted: u8,
    pub gps_ok_to_log: u8,
    pub gps_fix_ok: u8,
    pub gps_uart_fresh: u8,
    pub gps_rmc_fresh: u8,
    pub gps_gga_fresh: u8,
    pub gps_nav_fresh: u8,
    pub gps_clock_stale: u8,
    pub gps_data_fresh: u8,
    pub gps_ok: u8,
    pub gps_hist_drain_ok: u8,
    pub gps_log_reason: String,
    pub gps_rmc_last_parse_age_ms: u64,
    pub gps_gga_last_parse_age_ms: u64,
    pub gps_cross_check_rejects: u64,
    pub gps_stale_events: u64,
    pub gps_recovery_soft_attempts: u64,
    pub gps_recovery_soft_ok: u64,
    pub gps_recovery_hard_attempts: u64,
    pub gps_recovery_hard_ok: u64,
    pub gps_fix_quality: u8,
    pub gps_hdop: Raw,
    pub gps_satellites: Raw,
    pub gps_uart_last_rx_ms: u64,
    pub gps_uart_rx_idle_ms: u64,
    pub gps_uart_rx_bytes: u64,
    pub gps_nmea_parsed: u64,
    pub gps_nmea_checksum_bad: u64,
    pub gps_nmea_rmc_count: u64,
    pub gps_nmea_gga_count: u64,
    pub gps_last_rmc_nav_valid: u8,
    pub gps_rmc_interval_ms: u32,
    pub gps_rmc_rate_hz: Raw,
    pub gps_hist_rate_hz: Raw,
    pub sd_mounted: u8,
    pub sd_mount_attempts: u32,
    pub sd_mount_recovered: u32,
    pub sd_health_ok: u8,
    pub sd_health_writer_last_ok_ms: u64,
    pub sd_health_main_last_ok_ms: u64,
    pub sd_health_fail_count: u32,
    pub sd_health_last_fail_ms: u64,
    pub sd_session_created: u8,
    pub sd_append_us_last: u32,
    pub sd_append_us_max: u32,
    pub sd_writer_queue_full: u32,
    pub sd_io_rpc_timeouts: u32,
    pub sd_io_last_rpc_ms: u32,
    pub recon_sd_chunk_drops: u32,
    pub sd_eager_open_requested: u32,
    pub sd_eager_open_request_dropped: u32,
    pub sd_eager_open_succeeded: u32,
    pub sd_eager_open_failed: u32,
    pub sd_cleanup_files_examined: u32,
    pub sd_cleanup_files_deleted: u32,
    pub sd_cleanup_files_errors: u32,
    pub diag_log_lines_ok: u32,
    pub diag_log_lines_err: u32,
    pub diag_log_last_err_ms: u64,
    pub diag_log_enabled: u8,
    pub wifi_scan_cycles: u64,
    pub wifi_aps_seen_total: u64,
    pub wifi_rows_written: u64,
    pub wifi_rows_dropped_no_accuracy: u64,
    pub wifi_rows_dropped_no_gps_match: u64,
    pub wifi_rows_dropped_no_hdop: u64,
    pub wifi_rows_dropped_gps_stale: u64,
    pub ble_rows_dropped_gps_stale: u64,
    pub wifi_rows_dropped_dedup: u64,
    pub geodeduper_allowed: u64,
    pub geodeduper_suppressed: u64,
    pub geodeduper_evictions: u64,
    pub unique_wifi_aps: u64,
    pub unique_ble_devices: u64,
    pub mac_table_entries: u32,
    pub mac_table_geo_active: u32,
    pub mac_table_max_entries: u32,
    pub mac_table_geo_slots_max: u32,
    pub mac_table_row_evictions: u64,
    pub mac_table_geo_releases: u64,
    pub wifi_rows_dropped_denylist: u64,
    pub wifi_rows_dropped_encode: u64,
    pub wifi_rows_dropped_sd: u64,
    pub sniffer_watchdog_recover: u32,
    pub sniffer_watchdog_reboot: u32,
    pub wifi_batches_dropped_fleet_hold: u64,
    pub wifi_batches_dropped_home_geofence: u64,
    pub wifi_batches_dropped_csv_gate: u64,
    pub wifi_batches_dropped_wall_clock: u64,
    pub wifi_batches_dropped_no_fix_timeline: u64,
    pub wifi_batches_dropped_utc_encode: u64,
    pub ble_batches_dropped_fleet_hold: u64,
    pub ble_batches_dropped_home_geofence: u64,
    pub ble_batches_dropped_csv_gate: u64,
    pub ble_batches_dropped_wall_clock: u64,
    pub ble_batches_dropped_no_fix_timeline: u64,
    pub ble_batches_dropped_utc_encode: u64,
    pub espnow_rx_dropped_queue_full: u32,
    pub espnow_rx_dropped_mutex_busy: u32,
    pub espnow_rx_dropped_disconnected: u32,
    pub espnow_tx_failed: u32,
    pub sniffer_ack_tx_failed: u32,
    pub espnow_peer_mod_attempts: u32,
    pub espnow_peer_mod_failed: u32,
    pub espnow_periodic_ping_broadcast_fail: u32,
    pub control_wifi_softap_restore_count: u32,
    pub control_espnow_stack_refresh_ok: u32,
    pub control_espnow_stack_refresh_fail: u32,
    pub hello_tx_welcome_fail: u32,
    pub hello_tx_cmd_fail: u32,
    pub hello_tx_denylist_fail: u32,
    pub hello_tx_timeset_fail: u32,
    pub ingress_proto_reject: u32,
    pub ingress_hello_rx: u32,
    pub ingress_hello_unpack_fail: u32,
    pub ingress_hello_adopt_pending: u32,
    pub espnow_rx_cb_total: u32,
    pub espnow_rx_cb_no_sender: u32,
    pub control_ingest_frames_drained: u32,
    pub control_ingest_worker_batches: u32,
    pub control_ingest_stack_bytes: u32,
    pub control_loop_frames_drained_last: u32,
    pub control_main_loop_tick_ms: u32,
    pub control_main_loop_last_iter_ms: u32,
    pub control_main_loop_phase: u8,
    pub control_snapshot_build_ms: u32,
    pub control_snapshot_age_ms: u64,
    pub control_snapshot_write_skipped: u32,
    pub promisc_rx_dropped_queue_full: u32,
    pub sniffer_last_abort_pressure: u8,
    pub espnow_rx_queue_inflight: u32,
    pub espnow_rx_queue_hwm: u32,
    pub sd_writer_queue_inflight: u32,
    pub sd_writer_queue_hwm: u32,
    pub min_free_internal_heap_bytes: u32,
    pub min_free_spiram_heap_bytes: u32,
    pub soc_temp_c: Raw,
    pub upload_runs: u64,
    pub upload_phase: String,
    pub upload_files_done: u32,
    pub upload_files_total: u32,
    pub upload_run_wigle_ok: u32,
    pub upload_run_wigle_failed: u32,
    pub upload_run_wdgwars_ok: u32,
    pub upload_run_wdgwars_failed: u32,
    pub upload_last_wigle_ok: u32,
    pub upload_last_wigle_failed: u32,
    pub upload_last_wdgwars_ok: u32,
    pub upload_last_wdgwars_failed: u32,
    pub upload_pending_csv_count: u32,
    pub wigle_api_name: String,
    pub home_zone_lat: Raw,
    pub home_zone_lon: Raw,
    pub home_zone_radius_m: u32,
    pub home_zone_gps_line: String,
    pub wardrive_active: u8,
    pub fleet_phase: u8,
    pub fleet_phase_name: String,
    pub fleet_phase_entered_ms: u64,
    pub fleet_phase_transitions: u32,
    pub wigle_configured: u8,
    pub wdgwars_configured: u8,
    pub provisioning_complete: u8,
    pub alert_sound_mask: u32,
    pub alert_buzzer_volume_percent: u32,
    pub management_ap_ssid: String,
    pub fleet_management: u8,
    pub scan_allowed: u8,
    pub csv_enqueue_allowed: u8,
    pub home_geo_configured: u8,
    pub home_geofence_suppressing: u8,
    pub detection_probe_frags_rx: u32,
    pub detection_bursts_rx: u32,
    pub detection_alerts_fired: u32,
    pub lpr_camera_wifi_alerts_fired_method1: u32,
    pub lpr_camera_wifi_alerts_fired_method2: u32,
    pub lpr_camera_wifi_alerts_fired_method3: u32,
    pub lpr_camera_wifi_ie_sig_match_builtin_total: u32,
    pub ble_alerts_fired: u32,
    pub detection_followup_rows_written: u32,
    pub detection_alerted_macs_tracked: u32,
    pub detection_last_src: String,
    pub detection_last_seen_ms: u64,
    pub detection_last_sniffer_id: u32,
    pub detection_last_wildcard_count: u16,
    pub detection_last_channel: u8,
    pub detection_alerts_rx: u32,
    pub detection_last_rssi_min: i8,
    pub detection_last_rssi_max: i8,
    pub detection_last_rssi_avg_q8: i16,
    pub detection_last_oui_matched: u8,
    pub detection_last_signal_kind: u8,
    pub detection_last_detection_method: u8,
    pub detection_last_ble_company_id: u16,
    pub detection_disable_wifi_mask: u32,
    pub detection_disable_ble_mask: u32,
    pub ble_active_scan: bool,
    pub remote_id_disable_mask: u32,
    pub remote_id_alerts_rx: u32,
    pub remote_id_rows_written: u32,
    pub remote_id_last_src: String,
    pub remote_id_last_seen_ms: u64,
    pub remote_id_last_sniffer_id: u32,
    pub remote_id_last_channel: u8,
    pub remote_id_last_rssi: i8,
    pub remote_id_last_signal_kind: u8,
    pub remote_id_last_detection_method: u8,
    pub mobile_surveillance_alerts_fired: u32,
    pub mobile_surveillance_last_src: String,
    pub mobile_surveillance_last_seen_ms: u64,
    pub mobile_surveillance_last_sniffer_id: u32,
    pub mobile_surveillance_last_signal_kind: u8,
    pub mobile_surveillance_last_detection_method: u8,
    pub ssid_watch_alerts_fired: u32,
    pub ssid_watch_last_ssid: String,
    pub ssid_watch_last_bssid: String,
    pub ssid_watch_last_seen_ms: u64,
    pub ssid_watch_last_sniffer_id: u32,
    pub ssid_watch_last_channel: u8,
    pub ssid_watch_last_rssi: i8,
    pub espnow_rx_dropped_oversized: u32,
    pub recon_hold: u8,
    pub recon_active: u8,
    pub recon_session_id: u32,
    pub recon_frags_rx: u32,
    pub recon_pkts_written: u32,
    pub recon_reasm_fail: u32,
    pub recon_remaining_ms: u64,
    pub recon_elapsed_ms: u64,
    pub recon_output_path: String,
    pub fleet_max_sniffers: usize,
    pub sniffers_overflow: u32,
    /// Serialized `sniffers` array (the existing serde type; emitted verbatim).
    pub sniffers: Raw,
    pub peer_pool_join: u32,
    pub peer_pool_evict: u32,
    pub peer_pool_stale_liveness_evict: u32,
    pub peer_pool_cap: u32,
    pub peer_pool_transport_end: u32,
    pub peer_pool_grace: u32,
    pub post_dump_done_tx: u32,
    pub post_dump_done_tx_fail: u32,
    pub time_sync_ping_tx: u32,
    pub time_sync_pong_rx: u32,
    pub time_sync_rtt_reject: u32,
    pub time_sync_rtt_skipped_not_due: u32,
    pub peer_join_nack_tx: u32,
    pub peer_join_queue_expire: u32,
    pub peer_visit_arm_timeout: u32,
    pub peer_pool_active: u8,
    pub peer_slots_free: u8,
    pub peer_join_waiters: u8,
    pub wolfpack_dump_samples: u32,
    pub wolfpack_queue_ms_avg: u64,
    pub wolfpack_queue_ms_max: u32,
    pub wolfpack_bulk_xfer_ms_avg: u64,
    pub wolfpack_bulk_xfer_ms_max: u32,
    /// The `gps` object (existing serde `GpsSnapshot`).
    pub gps: GpsSnapshot,
    /// The `detection_last` object (existing serde type).
    pub detection_last: DetectionLastSnapshot,
    /// The `mobile_surveillance_last` object (existing serde type).
    pub mobile_surveillance_last: MobileSurveillanceLastSnapshot,
    /// The `ssid_watch_last` object (existing serde type).
    pub ssid_watch_last: SsidWatchLastSnapshot,
    /// The `diagnostics` object, built by the control side (Tier-B diagnostics).
    pub diagnostics: Raw,
}

/// Serialize the body to its compact JSON string.
pub fn api_stats_body_to_string(body: &ApiStatsBody) -> String {
    serde_json::to_string(body).expect("ApiStatsBody always serializes")
}

/// Inputs to [`build_api_stats_body`], grouped so the builder keeps a single argument.
///
/// This only regroups the caller-supplied values; it does **not** change the `/api/stats` wire.
/// [`ApiStatsBody`] (the serialized body) is untouched — field set, key order, and values are
/// byte-identical to before.
#[derive(Clone, Copy)]
pub struct StatsBodyInputs<'a> {
    pub wigle_api_name: &'a str,
    pub home_zone_poi_micro: Option<(i64, i64)>,
    pub home_zone_radius_m: u32,
    pub home_zone_configured: u8,
    pub home_zone_gps_line: &'a str,
    pub gps_fix: Option<GpsFixInput>,
    pub wall_clock_trusted: bool,
    pub fleet_phase_name: &'a str,
    pub recon_output_path: &'a str,
    pub board_profile: &'a str,
    pub firmware_version: &'a str,
    pub esp_idf_version: &'a str,
    pub management_ap_ssid: &'a str,
    pub peer_pool_active: u8,
    pub peer_slots_free: u8,
    pub peer_join_waiters: u8,
    pub diagnostics_json: &'a str,
    // Hardware values read by the control side (via `wardriver_shared`) so this builder
    // stays free of a `wardriver-shared` dependency.
    pub forms_rev: u32,
    pub control_cpu_load_pct: u8,
    pub reset_reason_str: &'a str,
    pub reset_reason_raw: u32,
    pub soc_temp_c_x10: Option<i32>,
}

/// Build the `/api/stats` body from the current runtime state.
///
/// Reads the global counters (same values `stats_json.rs` read), the tier-A snapshot
/// (for the nested objects + `scan_allowed` / `csv_enqueue_allowed` + `gps`), and the
/// control-supplied values (wigle API name, home-zone POI, GPS fix, peer-pool gauges,
/// diagnostics object, recon path, wall-clock trust).
pub fn build_api_stats_body(inputs: &StatsBodyInputs<'_>) -> ApiStatsBody {
    let StatsBodyInputs {
        wigle_api_name,
        home_zone_poi_micro,
        home_zone_radius_m,
        home_zone_configured,
        home_zone_gps_line,
        gps_fix,
        wall_clock_trusted,
        fleet_phase_name,
        recon_output_path,
        board_profile,
        firmware_version,
        esp_idf_version,
        management_ap_ssid,
        peer_pool_active,
        peer_slots_free,
        peer_join_waiters,
        diagnostics_json,
        forms_rev,
        control_cpu_load_pct,
        reset_reason_str,
        reset_reason_raw,
        soc_temp_c_x10,
    } = *inputs;
    use crate as stats;
    use std::sync::atomic::Ordering;

    let now_ms = stats::now_ms();
    let boot = stats::boot_ms();
    let uptime_ms = now_ms.saturating_sub(boot);
    let (wigle_configured, wdgwars_configured, provisioning_complete) = stats::nvs_config_flags();

    let tier_a = stats::build_runtime_snapshot_tier_a(now_ms, boot, gps_fix);
    let gps = &tier_a.gps;
    let gps_ok_to_log = if gps.fix_ok != 0 && gps.nav_fresh != 0 && gps.clock_stale == 0 {
        1
    } else {
        0
    };
    // Raw GPS fix quality as the caller observed it (`GpsFixInput`); `None` => 0.
    let fix_quality = gps_fix.map(|f| f.fix_quality).unwrap_or(0);

    let remote_id_last = stats::snapshot_remote_id_last();

    let (recon_end, recon_dur) = stats::recon_timing_ms();
    let (recon_remaining_ms, recon_elapsed_ms) = if recon_end != 0 && recon_dur != 0 {
        let rem = recon_end.saturating_sub(now_ms);
        let start = recon_end.saturating_sub(u64::from(recon_dur));
        let el = now_ms.saturating_sub(start).min(u64::from(recon_dur));
        (rem, el)
    } else {
        (0u64, 0u64)
    };

    let gps_uart_last_rx_ms = stats::gps_uart_last_rx_ms();
    let gps_uart_rx_idle_ms = if gps_uart_last_rx_ms == 0 {
        0
    } else {
        now_ms.saturating_sub(gps_uart_last_rx_ms)
    };

    let home_zone_lat = match home_zone_poi_micro {
        None => null_raw(),
        Some((lat, _)) => degrees_7_raw(lat),
    };
    let home_zone_lon = match home_zone_poi_micro {
        None => null_raw(),
        Some((_, lon)) => degrees_7_raw(lon),
    };

    let soc_temp_c = match soc_temp_c_x10 {
        None => null_raw(),
        Some(x10) => decimal_1_raw(x10),
    };

    let sniffers_json =
        serde_json::to_string(&tier_a.sniffers).unwrap_or_else(|_| "[]".to_string());

    let dump_samples = stats::WOLFPACK_DUMP_SAMPLES.load(Ordering::Relaxed);
    let queue_sum = stats::wolfpack_queue_ms_sum();
    let queue_max = stats::WOLFPACK_QUEUE_MS_MAX.load(Ordering::Relaxed);
    let bulk_sum = stats::wolfpack_bulk_xfer_ms_sum();
    let bulk_max = stats::WOLFPACK_BULK_XFER_MS_MAX.load(Ordering::Relaxed);
    let queue_avg = if dump_samples > 0 {
        queue_sum / u64::from(dump_samples)
    } else {
        0
    };
    let bulk_avg = if dump_samples > 0 {
        bulk_sum / u64::from(dump_samples)
    } else {
        0
    };

    ApiStatsBody {
        forms_rev,
        stats_schema_rev: tier_a.stats_schema_rev,
        firmware_version: firmware_version.to_string(),
        firmware_role: "control".to_string(),
        board_profile: board_profile.to_string(),
        proto_version: wardriver_proto::PROTO_VERSION as u32,
        esp_idf_version: esp_idf_version.to_string(),
        now_ms,
        uptime_ms,
        control_mem_heap_avail_pct: stats::heap_avail_pct(),
        control_mem_heap_free_bytes: stats::free_heap_internal_plus_spiram_bytes(),
        control_mem_heap_total_bytes: stats::total_heap_internal_plus_spiram_bytes(),
        control_cpu_load_pct,
        reset_reason: reset_reason_str.to_string(),
        reset_reason_raw,
        channel_plan_revision: stats::CHANNEL_PLAN_REVISION.load(Ordering::Acquire),
        channel_plan_sd_load_fail: stats::CHANNEL_PLAN_SD_LOAD_FAIL.load(Ordering::Acquire),
        channel_plan_sd_save_fail: stats::CHANNEL_PLAN_SD_SAVE_FAIL.load(Ordering::Acquire),
        gps_time_valid: stats::GPS_TIME_VALID.load(Ordering::Acquire) as u8,
        wall_clock_trusted: wall_clock_trusted as u8,
        gps_ok_to_log,
        gps_fix_ok: gps.fix_ok,
        gps_uart_fresh: gps.uart_fresh,
        gps_rmc_fresh: gps.rmc_fresh,
        gps_gga_fresh: gps.gga_fresh,
        gps_nav_fresh: gps.nav_fresh,
        gps_clock_stale: gps.clock_stale,
        gps_data_fresh: gps.data_fresh,
        gps_ok: gps_ok_to_log,
        gps_hist_drain_ok: stats::gps_hist_drain_ok() as u8,
        gps_log_reason: gps.log_reason.clone(),
        gps_rmc_last_parse_age_ms: stats::gps_rmc_last_parse_age_ms(now_ms).min(u32::MAX as u64),
        gps_gga_last_parse_age_ms: stats::gps_gga_last_parse_age_ms(now_ms).min(u32::MAX as u64),
        gps_cross_check_rejects: stats::gps_cross_check_rejects(),
        gps_stale_events: stats::gps_stale_events(),
        gps_recovery_soft_attempts: stats::gps_recovery_soft_attempts(),
        gps_recovery_soft_ok: stats::gps_recovery_soft_ok(),
        gps_recovery_hard_attempts: stats::gps_recovery_hard_attempts(),
        gps_recovery_hard_ok: stats::gps_recovery_hard_ok(),
        gps_fix_quality: fix_quality,
        gps_hdop: gps_hdop_raw(gps.hdop),
        gps_satellites: match gps.satellites {
            None => null_raw(),
            Some(n) => raw(&n.to_string()),
        },
        gps_uart_last_rx_ms,
        gps_uart_rx_idle_ms,
        gps_uart_rx_bytes: stats::gps_uart_rx_bytes(),
        gps_nmea_parsed: stats::gps_nmea_parsed(),
        gps_nmea_checksum_bad: stats::gps_nmea_checksum_bad_count(),
        gps_nmea_rmc_count: stats::gps_nmea_rmc_count(),
        gps_nmea_gga_count: stats::gps_nmea_gga_count(),
        gps_last_rmc_nav_valid: stats::GPS_LAST_RMC_NAV_VALID.load(Ordering::Acquire),
        gps_rmc_interval_ms: stats::gps_rmc_interval_ms(),
        gps_rmc_rate_hz: decimal_1_raw(stats::gps_rmc_rate_hz_x10() as i32),
        gps_hist_rate_hz: decimal_1_raw(stats::gps_hist_rate_hz_x10() as i32),
        sd_mounted: stats::SD_MOUNT_OK.load(Ordering::Acquire) as u8,
        sd_mount_attempts: stats::SD_MOUNT_ATTEMPTS.load(Ordering::Relaxed),
        sd_mount_recovered: stats::SD_MOUNT_RECOVERED.load(Ordering::Relaxed),
        sd_health_ok: stats::sd_health_ok() as u8,
        sd_health_writer_last_ok_ms: stats::sd_health_writer_last_ok_ms(),
        sd_health_main_last_ok_ms: stats::sd_health_main_last_ok_ms(),
        sd_health_fail_count: stats::sd_health_fail_count(),
        sd_health_last_fail_ms: stats::sd_health_last_fail_ms(),
        sd_session_created: stats::SD_SESSION_CREATED.load(Ordering::Acquire) as u8,
        sd_append_us_last: stats::sd_append_us_last(),
        sd_append_us_max: stats::sd_append_us_max(),
        sd_writer_queue_full: stats::sd_writer_queue_full(),
        sd_io_rpc_timeouts: stats::sd_io_rpc_timeouts(),
        sd_io_last_rpc_ms: stats::sd_io_last_rpc_ms(),
        recon_sd_chunk_drops: stats::recon_sd_chunk_drops(),
        sd_eager_open_requested: stats::sd_eager_open_requested_count(),
        sd_eager_open_request_dropped: stats::sd_eager_open_request_dropped_count(),
        sd_eager_open_succeeded: stats::sd_eager_open_succeeded_count(),
        sd_eager_open_failed: stats::sd_eager_open_failed_count(),
        sd_cleanup_files_examined: stats::sd_cleanup_files_examined(),
        sd_cleanup_files_deleted: stats::sd_cleanup_files_deleted(),
        sd_cleanup_files_errors: stats::sd_cleanup_files_errors(),
        diag_log_lines_ok: stats::diag_log_lines_ok(),
        diag_log_lines_err: stats::diag_log_lines_err(),
        diag_log_last_err_ms: stats::diag_log_last_err_ms(),
        diag_log_enabled: stats::diag_log_enabled() as u8,
        wifi_scan_cycles: stats::wifi_scan_cycles(),
        wifi_aps_seen_total: stats::wifi_aps_seen_total(),
        wifi_rows_written: stats::wifi_rows_written(),
        wifi_rows_dropped_no_accuracy: stats::wifi_rows_dropped_no_accuracy(),
        wifi_rows_dropped_no_gps_match: stats::wifi_rows_dropped_no_gps_match(),
        wifi_rows_dropped_no_hdop: stats::wifi_rows_dropped_no_hdop(),
        wifi_rows_dropped_gps_stale: stats::wifi_rows_dropped_gps_stale(),
        ble_rows_dropped_gps_stale: stats::ble_rows_dropped_gps_stale(),
        wifi_rows_dropped_dedup: stats::wifi_rows_dropped_dedup(),
        geodeduper_allowed: stats::geodeduper_allowed(),
        geodeduper_suppressed: stats::geodeduper_suppressed(),
        geodeduper_evictions: stats::geodeduper_evictions(),
        unique_wifi_aps: stats::unique_wifi_aps(),
        unique_ble_devices: stats::unique_ble_devices(),
        mac_table_entries: stats::MAC_TABLE_ENTRIES.load(Ordering::Relaxed),
        mac_table_geo_active: stats::MAC_TABLE_GEO_ACTIVE.load(Ordering::Relaxed),
        mac_table_max_entries: stats::MAC_TABLE_MAX_ENTRIES.load(Ordering::Relaxed),
        mac_table_geo_slots_max: stats::MAC_TABLE_GEO_SLOTS_MAX.load(Ordering::Relaxed),
        mac_table_row_evictions: stats::mac_table_row_evictions(),
        mac_table_geo_releases: stats::mac_table_geo_releases(),
        wifi_rows_dropped_denylist: stats::wifi_rows_dropped_denylist(),
        wifi_rows_dropped_encode: stats::wifi_rows_dropped_encode(),
        wifi_rows_dropped_sd: stats::wifi_rows_dropped_sd(),
        sniffer_watchdog_recover: stats::SNIFFER_WATCHDOG_RECOVER.load(Ordering::Relaxed),
        sniffer_watchdog_reboot: stats::SNIFFER_WATCHDOG_REBOOT.load(Ordering::Relaxed),
        wifi_batches_dropped_fleet_hold: stats::wifi_batches_dropped_fleet_hold(),
        wifi_batches_dropped_home_geofence: stats::wifi_batches_dropped_home_geofence(),
        wifi_batches_dropped_csv_gate: stats::wifi_batches_dropped_csv_gate(),
        wifi_batches_dropped_wall_clock: stats::wifi_batches_dropped_wall_clock(),
        wifi_batches_dropped_no_fix_timeline: stats::wifi_batches_dropped_no_fix_timeline(),
        wifi_batches_dropped_utc_encode: stats::wifi_batches_dropped_utc_encode(),
        ble_batches_dropped_fleet_hold: stats::ble_batches_dropped_fleet_hold(),
        ble_batches_dropped_home_geofence: stats::ble_batches_dropped_home_geofence(),
        ble_batches_dropped_csv_gate: stats::ble_batches_dropped_csv_gate(),
        ble_batches_dropped_wall_clock: stats::ble_batches_dropped_wall_clock(),
        ble_batches_dropped_no_fix_timeline: stats::ble_batches_dropped_no_fix_timeline(),
        ble_batches_dropped_utc_encode: stats::ble_batches_dropped_utc_encode(),
        espnow_rx_dropped_queue_full: stats::espnow_rx_dropped_queue_full(),
        espnow_rx_dropped_mutex_busy: stats::espnow_rx_dropped_mutex_busy(),
        espnow_rx_dropped_disconnected: stats::espnow_rx_dropped_disconnected(),
        espnow_tx_failed: stats::espnow_tx_failed(),
        sniffer_ack_tx_failed: stats::sniffer_ack_tx_failed(),
        espnow_peer_mod_attempts: stats::espnow_peer_mod_attempts(),
        espnow_peer_mod_failed: stats::espnow_peer_mod_failed(),
        espnow_periodic_ping_broadcast_fail: stats::ESPNOW_PERIODIC_PING_BROADCAST_FAIL
            .load(Ordering::Relaxed),
        control_wifi_softap_restore_count: stats::CONTROL_WIFI_SOFTAP_RESTORE_COUNT
            .load(Ordering::Relaxed),
        control_espnow_stack_refresh_ok: stats::CONTROL_ESPNOW_STACK_REFRESH_OK
            .load(Ordering::Relaxed),
        control_espnow_stack_refresh_fail: stats::CONTROL_ESPNOW_STACK_REFRESH_FAIL
            .load(Ordering::Relaxed),
        hello_tx_welcome_fail: stats::HELLO_TX_WELCOME_FAIL.load(Ordering::Relaxed),
        hello_tx_cmd_fail: stats::HELLO_TX_CMD_FAIL.load(Ordering::Relaxed),
        hello_tx_denylist_fail: stats::HELLO_TX_DENYLIST_FAIL.load(Ordering::Relaxed),
        hello_tx_timeset_fail: stats::HELLO_TX_TIMESET_FAIL.load(Ordering::Relaxed),
        ingress_proto_reject: stats::INGRESS_PROTO_REJECT.load(Ordering::Relaxed),
        ingress_hello_rx: stats::INGRESS_HELLO_RX.load(Ordering::Relaxed),
        ingress_hello_unpack_fail: stats::INGRESS_HELLO_UNPACK_FAIL.load(Ordering::Relaxed),
        ingress_hello_adopt_pending: stats::INGRESS_HELLO_ADOPT_PENDING.load(Ordering::Relaxed),
        espnow_rx_cb_total: stats::ESPNOW_RX_CB_TOTAL.load(Ordering::Relaxed),
        espnow_rx_cb_no_sender: stats::ESPNOW_RX_CB_NO_SENDER.load(Ordering::Relaxed),
        control_ingest_frames_drained: stats::CONTROL_INGEST_FRAMES_DRAINED.load(Ordering::Relaxed),
        control_ingest_worker_batches: stats::CONTROL_INGEST_WORKER_BATCHES.load(Ordering::Relaxed),
        control_ingest_stack_bytes: stats::CONTROL_INGEST_STACK_BYTES.load(Ordering::Relaxed),
        control_loop_frames_drained_last: stats::CONTROL_LOOP_FRAMES_DRAINED_LAST
            .load(Ordering::Relaxed),
        control_main_loop_tick_ms: stats::control_main_loop_tick_ms(),
        control_main_loop_last_iter_ms: stats::control_main_loop_last_iter_ms(),
        control_main_loop_phase: stats::control_main_loop_phase(),
        control_snapshot_build_ms: stats::control_snapshot_build_ms(),
        control_snapshot_age_ms: stats::control_snapshot_age_ms(now_ms),
        control_snapshot_write_skipped: stats::control_snapshot_write_skipped(),
        promisc_rx_dropped_queue_full: stats::promisc_rx_dropped_queue_full(),
        sniffer_last_abort_pressure: stats::sniffer_last_abort_pressure(),
        espnow_rx_queue_inflight: stats::ESPNOW_RX_QUEUE_INFLIGHT.load(Ordering::Relaxed),
        espnow_rx_queue_hwm: stats::ESPNOW_RX_QUEUE_HWM.load(Ordering::Relaxed),
        sd_writer_queue_inflight: stats::SD_WRITER_QUEUE_INFLIGHT.load(Ordering::Relaxed),
        sd_writer_queue_hwm: stats::SD_WRITER_QUEUE_HWM.load(Ordering::Relaxed),
        min_free_internal_heap_bytes: stats::MIN_FREE_INTERNAL_HEAP_BYTES.load(Ordering::Relaxed),
        min_free_spiram_heap_bytes: stats::MIN_FREE_SPIRAM_HEAP_BYTES.load(Ordering::Relaxed),
        soc_temp_c,
        upload_runs: stats::upload_runs(),
        upload_phase: stats::upload_phase_name(stats::upload_phase()).to_string(),
        upload_files_done: stats::UPLOAD_FILES_DONE.load(Ordering::Acquire),
        upload_files_total: stats::UPLOAD_FILES_TOTAL.load(Ordering::Acquire),
        upload_run_wigle_ok: stats::UPLOAD_RUN_WIGLE_OK.load(Ordering::Acquire),
        upload_run_wigle_failed: stats::UPLOAD_RUN_WIGLE_FAILED.load(Ordering::Acquire),
        upload_run_wdgwars_ok: stats::UPLOAD_RUN_WDGWARS_OK.load(Ordering::Acquire),
        upload_run_wdgwars_failed: stats::UPLOAD_RUN_WDGWARS_FAILED.load(Ordering::Acquire),
        upload_last_wigle_ok: stats::UPLOAD_LAST_WIGLE_OK.load(Ordering::Acquire),
        upload_last_wigle_failed: stats::UPLOAD_LAST_WIGLE_FAILED.load(Ordering::Acquire),
        upload_last_wdgwars_ok: stats::UPLOAD_LAST_WDGWARS_OK.load(Ordering::Acquire),
        upload_last_wdgwars_failed: stats::UPLOAD_LAST_WDGWARS_FAILED.load(Ordering::Acquire),
        upload_pending_csv_count: stats::upload_pending_csv_count(),
        wigle_api_name: wigle_api_name.to_string(),
        home_zone_lat,
        home_zone_lon,
        home_zone_radius_m,
        home_zone_gps_line: home_zone_gps_line.to_string(),
        wardrive_active: stats::wardrive_active() as u8,
        fleet_phase: stats::fleet_phase(),
        fleet_phase_name: fleet_phase_name.to_string(),
        fleet_phase_entered_ms: stats::fleet_phase_entered_ms(),
        fleet_phase_transitions: stats::fleet_phase_transitions(),
        wigle_configured: wigle_configured as u8,
        wdgwars_configured: wdgwars_configured as u8,
        provisioning_complete: provisioning_complete as u8,
        alert_sound_mask: stats::alert_sound_mask() as u32,
        alert_buzzer_volume_percent: stats::alert_buzzer_volume_percent() as u32,
        management_ap_ssid: management_ap_ssid.to_string(),
        fleet_management: stats::fleet_management_hold() as u8,
        scan_allowed: tier_a.scan_allowed,
        csv_enqueue_allowed: tier_a.csv_enqueue_allowed,
        home_geo_configured: home_zone_configured,
        home_geofence_suppressing: stats::HOME_GEOFENCE_SUPPRESSING.load(Ordering::Acquire) as u8,
        detection_probe_frags_rx: stats::DETECTION_PROBE_FRAGS_RX.load(Ordering::Relaxed),
        detection_bursts_rx: stats::DETECTION_BURSTS_RX.load(Ordering::Relaxed),
        detection_alerts_fired: stats::DETECTION_ALERTS_FIRED.load(Ordering::Relaxed),
        lpr_camera_wifi_alerts_fired_method1: stats::DETECTION_WIFI_ALERTS_FIRED_METHOD1
            .load(Ordering::Relaxed),
        lpr_camera_wifi_alerts_fired_method2: stats::DETECTION_WIFI_ALERTS_FIRED_METHOD2
            .load(Ordering::Relaxed),
        lpr_camera_wifi_alerts_fired_method3: stats::DETECTION_WIFI_ALERTS_FIRED_METHOD3
            .load(Ordering::Relaxed),
        lpr_camera_wifi_ie_sig_match_builtin_total:
            stats::DETECTION_WIFI_IE_SIG_MATCH_BUILTIN_TOTAL.load(Ordering::Relaxed),
        ble_alerts_fired: stats::BLE_ALERTS_FIRED.load(Ordering::Relaxed),
        detection_followup_rows_written: stats::DETECTION_FOLLOWUP_ROWS_WRITTEN
            .load(Ordering::Relaxed),
        detection_alerted_macs_tracked: stats::DETECTION_ALERTED_MACS_TRACKED
            .load(Ordering::Relaxed),
        detection_last_src: tier_a.detection_last.src.clone(),
        detection_last_seen_ms: tier_a.detection_last.last_seen_ms,
        detection_last_sniffer_id: tier_a.detection_last.sniffer_id,
        detection_last_wildcard_count: tier_a.detection_last.wildcard_count,
        detection_last_channel: tier_a.detection_last.channel,
        detection_alerts_rx: stats::DETECTION_ALERTS_RX.load(Ordering::Relaxed),
        detection_last_rssi_min: tier_a.detection_last.rssi_min,
        detection_last_rssi_max: tier_a.detection_last.rssi_max,
        detection_last_rssi_avg_q8: tier_a.detection_last.rssi_avg_q8,
        detection_last_oui_matched: tier_a.detection_last.oui_matched,
        detection_last_signal_kind: tier_a.detection_last.signal_kind,
        detection_last_detection_method: tier_a.detection_last.detection_method,
        detection_last_ble_company_id: tier_a.detection_last.ble_company_id,
        detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
        detection_disable_ble_mask: stats::detection_disable_ble_mask(),
        ble_active_scan: stats::ble_active_scan(),
        remote_id_disable_mask: stats::remote_id_disable_mask(),
        remote_id_alerts_rx: stats::REMOTE_ID_ALERTS_RX.load(Ordering::Relaxed),
        remote_id_rows_written: stats::REMOTE_ID_ROWS_WRITTEN.load(Ordering::Relaxed),
        remote_id_last_src: wardriver_core::wigle::format_bssid(&remote_id_last.src),
        remote_id_last_seen_ms: remote_id_last.last_seen_ms,
        remote_id_last_sniffer_id: remote_id_last.sniffer_id,
        remote_id_last_channel: remote_id_last.channel,
        remote_id_last_rssi: remote_id_last.rssi,
        remote_id_last_signal_kind: remote_id_last.signal_kind,
        remote_id_last_detection_method: remote_id_last.detection_method,
        mobile_surveillance_alerts_fired: stats::MOBILE_SURVEILLANCE_ALERTS_FIRED
            .load(Ordering::Relaxed),
        mobile_surveillance_last_src: tier_a.mobile_surveillance_last.src.clone(),
        mobile_surveillance_last_seen_ms: tier_a.mobile_surveillance_last.last_seen_ms,
        mobile_surveillance_last_sniffer_id: tier_a.mobile_surveillance_last.sniffer_id,
        mobile_surveillance_last_signal_kind: tier_a.mobile_surveillance_last.signal_kind,
        mobile_surveillance_last_detection_method: tier_a.mobile_surveillance_last.detection_method,
        ssid_watch_alerts_fired: stats::SSID_WATCH_ALERTS_FIRED.load(Ordering::Relaxed),
        ssid_watch_last_ssid: tier_a.ssid_watch_last.ssid.clone(),
        ssid_watch_last_bssid: tier_a.ssid_watch_last.bssid.clone(),
        ssid_watch_last_seen_ms: tier_a.ssid_watch_last.last_seen_ms,
        ssid_watch_last_sniffer_id: tier_a.ssid_watch_last.sniffer_id,
        ssid_watch_last_channel: tier_a.ssid_watch_last.channel,
        ssid_watch_last_rssi: tier_a.ssid_watch_last.rssi,
        espnow_rx_dropped_oversized: stats::espnow_rx_dropped_oversized(),
        recon_hold: stats::recon_hold() as u8,
        recon_active: stats::RECON_ACTIVE.load(Ordering::Acquire),
        recon_session_id: stats::RECON_SESSION_ID.load(Ordering::Acquire),
        recon_frags_rx: stats::RECON_FRAGS_RX.load(Ordering::Relaxed),
        recon_pkts_written: stats::RECON_PKTS_WRITTEN.load(Ordering::Relaxed),
        recon_reasm_fail: stats::RECON_REASM_FAIL.load(Ordering::Relaxed),
        recon_remaining_ms,
        recon_elapsed_ms,
        recon_output_path: recon_output_path.to_string(),
        fleet_max_sniffers: fleet::FLEET_MAX_SNIFFERS,
        sniffers_overflow: stats::sniffers_overflow(),
        sniffers: raw(&sniffers_json),
        peer_pool_join: stats::PEER_POOL_JOIN.load(Ordering::Relaxed),
        peer_pool_evict: stats::PEER_POOL_EVICT.load(Ordering::Relaxed),
        peer_pool_stale_liveness_evict: stats::PEER_POOL_STALE_LIVENESS_EVICT
            .load(Ordering::Relaxed),
        peer_pool_cap: stats::PEER_POOL_CAP.load(Ordering::Relaxed),
        peer_pool_transport_end: stats::PEER_POOL_TRANSPORT_END.load(Ordering::Relaxed),
        peer_pool_grace: stats::PEER_POOL_GRACE.load(Ordering::Relaxed),
        post_dump_done_tx: stats::POST_DUMP_DONE_TX.load(Ordering::Relaxed),
        post_dump_done_tx_fail: stats::POST_DUMP_DONE_TX_FAIL.load(Ordering::Relaxed),
        time_sync_ping_tx: stats::TIME_SYNC_PING_TX.load(Ordering::Relaxed),
        time_sync_pong_rx: stats::TIME_SYNC_PONG_RX.load(Ordering::Relaxed),
        time_sync_rtt_reject: stats::TIME_SYNC_RTT_REJECT.load(Ordering::Relaxed),
        time_sync_rtt_skipped_not_due: stats::TIME_SYNC_RTT_SKIPPED_NOT_DUE.load(Ordering::Relaxed),
        peer_join_nack_tx: stats::PEER_JOIN_NACK_TX.load(Ordering::Relaxed),
        peer_join_queue_expire: stats::PEER_JOIN_QUEUE_EXPIRE.load(Ordering::Relaxed),
        peer_visit_arm_timeout: stats::PEER_VISIT_ARM_TIMEOUT.load(Ordering::Relaxed),
        peer_pool_active,
        peer_slots_free,
        peer_join_waiters,
        wolfpack_dump_samples: dump_samples,
        wolfpack_queue_ms_avg: queue_avg,
        wolfpack_queue_ms_max: queue_max,
        wolfpack_bulk_xfer_ms_avg: bulk_avg,
        wolfpack_bulk_xfer_ms_max: bulk_max,
        gps: tier_a.gps.clone(),
        detection_last: tier_a.detection_last.clone(),
        mobile_surveillance_last: tier_a.mobile_surveillance_last.clone(),
        ssid_watch_last: tier_a.ssid_watch_last.clone(),
        diagnostics: raw(diagnostics_json),
    }
}
