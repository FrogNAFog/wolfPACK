//! Golden test for the single-sourced `/api/stats` body.
//!
//! Builds a fully-deterministic [`wardriver_stats::ApiStatsBody`] from known constants
//! (no live hardware) and asserts that the serialized key/value set matches the captured
//! fixture exactly, **except** for the single removed key `detection_wildcard_window_ms`.
//!
//! The fixture is generated from the same constants via:
//!
//! ```sh
//! GEN_STATS_FIXTURE=1 cargo test-host -p wardriver-stats --test stats_body_golden
//! ```
//!
//! After generation the fixture is committed; the golden test then asserts the builder
//! reproduces it (modulo the one allowed removal). Any future builder change that alters
//! a key or value breaks this test.

use serde_json::Value;
use std::path::Path;

use wardriver_stats::{
    api_stats_body_to_string, ApiStatsBody, DetectionLastSnapshot, GpsSnapshot,
    MobileSurveillanceLastSnapshot, SsidWatchLastSnapshot,
};

/// The one and only key removed by the single-source refactor.
const REMOVED_KEY: &str = "detection_wildcard_window_ms";

/// Build a deterministic body from known constants. Every field is set to a fixed value
/// so the serialized output is reproducible.
fn build_golden_body() -> ApiStatsBody {
    // `Raw` fields are preformatted JSON fragments (the no-`core::fmt`-float paths).
    let raw = |s: &str| -> Box<serde_json::value::RawValue> {
        serde_json::value::RawValue::from_string(s.to_string()).unwrap()
    };

    ApiStatsBody {
        forms_rev: 7,
        stats_schema_rev: 2,
        firmware_version: "0.1.0".to_string(),
        firmware_role: "control".to_string(),
        board_profile: "xiao".to_string(),
        proto_version: 6,
        esp_idf_version: "v5.3".to_string(),
        now_ms: 1_700_000_000_000,
        uptime_ms: 3_600_000,
        control_mem_heap_avail_pct: 42,
        control_mem_heap_free_bytes: 120_000,
        control_mem_heap_total_bytes: 300_000,
        control_cpu_load_pct: 7,
        reset_reason: "POWERON".to_string(),
        reset_reason_raw: 1,
        channel_plan_revision: 3,
        channel_plan_sd_load_fail: 0,
        channel_plan_sd_save_fail: 0,
        gps_time_valid: 1,
        wall_clock_trusted: 1,
        gps_ok_to_log: 1,
        gps_fix_ok: 1,
        gps_uart_fresh: 1,
        gps_rmc_fresh: 1,
        gps_gga_fresh: 1,
        gps_nav_fresh: 1,
        gps_clock_stale: 0,
        gps_data_fresh: 1,
        gps_ok: 1,
        gps_hist_drain_ok: 1,
        gps_log_reason: "fix-ok".to_string(),
        gps_rmc_last_parse_age_ms: 120,
        gps_gga_last_parse_age_ms: 110,
        gps_cross_check_rejects: 0,
        gps_stale_events: 0,
        gps_recovery_soft_attempts: 0,
        gps_recovery_soft_ok: 0,
        gps_recovery_hard_attempts: 0,
        gps_recovery_hard_ok: 0,
        gps_fix_quality: 3,
        gps_hdop: raw("1.25"),
        gps_satellites: raw("12"),
        gps_uart_last_rx_ms: 1_700_000_000_000 - 120,
        gps_uart_rx_idle_ms: 120,
        gps_uart_rx_bytes: 5000,
        gps_nmea_parsed: 900,
        gps_nmea_checksum_bad: 0,
        gps_nmea_rmc_count: 450,
        gps_nmea_gga_count: 450,
        gps_last_rmc_nav_valid: 1,
        gps_rmc_interval_ms: 1000,
        gps_rmc_rate_hz: raw("1.0"),
        gps_hist_rate_hz: raw("1.0"),
        sd_mounted: 1,
        sd_mount_attempts: 1,
        sd_mount_recovered: 0,
        sd_health_ok: 1,
        sd_health_writer_last_ok_ms: 1_700_000_000_000 - 500,
        sd_health_main_last_ok_ms: 1_700_000_000_000 - 500,
        sd_health_fail_count: 0,
        sd_health_last_fail_ms: 0,
        sd_session_created: 1,
        sd_append_us_last: 250,
        sd_append_us_max: 400,
        sd_writer_queue_full: 0,
        sd_io_rpc_timeouts: 0,
        sd_io_last_rpc_ms: 15,
        recon_sd_chunk_drops: 0,
        sd_eager_open_requested: 0,
        sd_eager_open_request_dropped: 0,
        sd_eager_open_succeeded: 0,
        sd_eager_open_failed: 0,
        sd_cleanup_files_examined: 0,
        sd_cleanup_files_deleted: 0,
        sd_cleanup_files_errors: 0,
        diag_log_lines_ok: 10,
        diag_log_lines_err: 0,
        diag_log_last_err_ms: 0,
        diag_log_enabled: 1,
        wifi_scan_cycles: 50,
        wifi_aps_seen_total: 200,
        wifi_rows_written: 180,
        wifi_rows_dropped_no_accuracy: 0,
        wifi_rows_dropped_no_gps_match: 0,
        wifi_rows_dropped_no_hdop: 0,
        wifi_rows_dropped_gps_stale: 0,
        ble_rows_dropped_gps_stale: 0,
        wifi_rows_dropped_dedup: 20,
        geodeduper_allowed: 150,
        geodeduper_suppressed: 30,
        geodeduper_evictions: 0,
        unique_wifi_aps: 180,
        unique_ble_devices: 5,
        mac_table_entries: 60,
        mac_table_geo_active: 55,
        mac_table_max_entries: 512,
        mac_table_geo_slots_max: 512,
        mac_table_row_evictions: 0,
        mac_table_geo_releases: 0,
        wifi_rows_dropped_denylist: 0,
        wifi_rows_dropped_encode: 0,
        wifi_rows_dropped_sd: 0,
        sniffer_watchdog_recover: 0,
        sniffer_watchdog_reboot: 0,
        wifi_batches_dropped_fleet_hold: 0,
        wifi_batches_dropped_home_geofence: 0,
        wifi_batches_dropped_csv_gate: 0,
        wifi_batches_dropped_wall_clock: 0,
        wifi_batches_dropped_no_fix_timeline: 0,
        wifi_batches_dropped_utc_encode: 0,
        ble_batches_dropped_fleet_hold: 0,
        ble_batches_dropped_home_geofence: 0,
        ble_batches_dropped_csv_gate: 0,
        ble_batches_dropped_wall_clock: 0,
        ble_batches_dropped_no_fix_timeline: 0,
        ble_batches_dropped_utc_encode: 0,
        espnow_rx_dropped_queue_full: 0,
        espnow_rx_dropped_mutex_busy: 0,
        espnow_rx_dropped_disconnected: 0,
        espnow_tx_failed: 0,
        sniffer_ack_tx_failed: 0,
        espnow_peer_mod_attempts: 0,
        espnow_peer_mod_failed: 0,
        espnow_periodic_ping_broadcast_fail: 0,
        control_wifi_softap_restore_count: 0,
        control_espnow_stack_refresh_ok: 0,
        control_espnow_stack_refresh_fail: 0,
        hello_tx_welcome_fail: 0,
        hello_tx_cmd_fail: 0,
        hello_tx_denylist_fail: 0,
        hello_tx_timeset_fail: 0,
        ingress_proto_reject: 0,
        ingress_hello_rx: 4,
        ingress_hello_unpack_fail: 0,
        ingress_hello_adopt_pending: 0,
        espnow_rx_cb_total: 100,
        espnow_rx_cb_no_sender: 0,
        control_ingest_frames_drained: 100,
        control_ingest_worker_batches: 20,
        control_ingest_stack_bytes: 8192,
        control_loop_frames_drained_last: 1,
        control_main_loop_tick_ms: 1000,
        control_main_loop_last_iter_ms: 35,
        control_main_loop_phase: 0,
        control_snapshot_build_ms: 4,
        control_snapshot_age_ms: 100,
        control_snapshot_write_skipped: 0,
        promisc_rx_dropped_queue_full: 0,
        sniffer_last_abort_pressure: 0,
        espnow_rx_queue_inflight: 0,
        espnow_rx_queue_hwm: 2,
        sd_writer_queue_inflight: 0,
        sd_writer_queue_hwm: 3,
        min_free_internal_heap_bytes: 90_000,
        min_free_spiram_heap_bytes: 0,
        soc_temp_c: raw("33.5"),
        upload_runs: 0,
        upload_phase: "idle".to_string(),
        upload_files_done: 0,
        upload_files_total: 0,
        upload_run_wigle_ok: 0,
        upload_run_wigle_failed: 0,
        upload_run_wdgwars_ok: 0,
        upload_run_wdgwars_failed: 0,
        upload_last_wigle_ok: 0,
        upload_last_wigle_failed: 0,
        upload_last_wdgwars_ok: 0,
        upload_last_wdgwars_failed: 0,
        upload_pending_csv_count: 0,
        wigle_api_name: "api.wigle.net".to_string(),
        home_zone_lat: raw("37.1234567"),
        home_zone_lon: raw("-122.9876543"),
        home_zone_radius_m: 250,
        home_zone_gps_line: "37.1234567,-122.9876543".to_string(),
        wardrive_active: 1,
        fleet_phase: 0,
        fleet_phase_name: "scanning".to_string(),
        fleet_phase_entered_ms: 1_700_000_000_000 - 3_600_000,
        fleet_phase_transitions: 1,
        wigle_configured: 1,
        wdgwars_configured: 0,
        provisioning_complete: 1,
        alert_sound_mask: 0b0011,
        alert_buzzer_volume_percent: 50,
        management_ap_ssid: "Fleet-Mgmt".to_string(),
        fleet_management: 0,
        scan_allowed: 1,
        csv_enqueue_allowed: 1,
        home_geo_configured: 1,
        home_geofence_suppressing: 0,
        detection_probe_frags_rx: 0,
        detection_bursts_rx: 0,
        detection_alerts_fired: 0,
        lpr_camera_wifi_alerts_fired_method1: 0,
        lpr_camera_wifi_alerts_fired_method2: 0,
        lpr_camera_wifi_alerts_fired_method3: 0,
        lpr_camera_wifi_ie_sig_match_builtin_total: 0,
        ble_alerts_fired: 0,
        detection_followup_rows_written: 0,
        detection_alerted_macs_tracked: 0,
        detection_last_src: "AA:BB:CC:DD:EE:FF".to_string(),
        detection_last_seen_ms: 0,
        detection_last_sniffer_id: 0,
        detection_last_wildcard_count: 0,
        detection_last_channel: 0,
        detection_alerts_rx: 0,
        detection_last_rssi_min: 0,
        detection_last_rssi_max: 0,
        detection_last_rssi_avg_q8: 0,
        detection_last_oui_matched: 0,
        detection_last_signal_kind: 0,
        detection_last_detection_method: 0,
        detection_last_ble_company_id: 0,
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ble_active_scan: false,
        remote_id_disable_mask: 0,
        remote_id_alerts_rx: 0,
        remote_id_rows_written: 0,
        remote_id_last_src: "00:11:22:33:44:55".to_string(),
        remote_id_last_seen_ms: 0,
        remote_id_last_sniffer_id: 0,
        remote_id_last_channel: 0,
        remote_id_last_rssi: 0,
        remote_id_last_signal_kind: 0,
        remote_id_last_detection_method: 0,
        mobile_surveillance_alerts_fired: 0,
        mobile_surveillance_last_src: "66:77:88:99:AA:BB".to_string(),
        mobile_surveillance_last_seen_ms: 0,
        mobile_surveillance_last_sniffer_id: 0,
        mobile_surveillance_last_signal_kind: 0,
        mobile_surveillance_last_detection_method: 0,
        ssid_watch_alerts_fired: 0,
        ssid_watch_last_ssid: "HiddenAP".to_string(),
        ssid_watch_last_bssid: "CC:DD:EE:FF:00:11".to_string(),
        ssid_watch_last_seen_ms: 0,
        ssid_watch_last_sniffer_id: 0,
        ssid_watch_last_channel: 6,
        ssid_watch_last_rssi: -65,
        espnow_rx_dropped_oversized: 0,
        recon_hold: 0,
        recon_active: 0,
        recon_session_id: 0,
        recon_frags_rx: 0,
        recon_pkts_written: 0,
        recon_reasm_fail: 0,
        recon_remaining_ms: 0,
        recon_elapsed_ms: 0,
        recon_output_path: "/sdcard/recon.bin".to_string(),
        fleet_max_sniffers: 48,
        sniffers_overflow: 0,
        sniffers: raw("[]"),
        peer_pool_join: 0,
        peer_pool_evict: 0,
        peer_pool_stale_liveness_evict: 0,
        peer_pool_cap: 0,
        peer_pool_transport_end: 0,
        peer_pool_grace: 0,
        post_dump_done_tx: 0,
        post_dump_done_tx_fail: 0,
        time_sync_ping_tx: 0,
        time_sync_pong_rx: 0,
        time_sync_rtt_reject: 0,
        time_sync_rtt_skipped_not_due: 0,
        peer_join_nack_tx: 0,
        peer_join_queue_expire: 0,
        peer_visit_arm_timeout: 0,
        peer_pool_active: 0,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        wolfpack_dump_samples: 0,
        wolfpack_queue_ms_avg: 0,
        wolfpack_queue_ms_max: 0,
        wolfpack_bulk_xfer_ms_avg: 0,
        wolfpack_bulk_xfer_ms_max: 0,
        gps: GpsSnapshot {
            fix_ok: 1,
            hdop: Some(1.25),
            satellites: Some(12),
            uart_fresh: 1,
            rmc_fresh: 1,
            gga_fresh: 1,
            nav_fresh: 1,
            clock_stale: 0,
            data_fresh: 1,
            log_reason: "fix-ok".to_string(),
        },
        detection_last: DetectionLastSnapshot {
            last_seen_ms: 0,
            sniffer_id: 0,
            src: "AA:BB:CC:DD:EE:FF".to_string(),
            wildcard_count: 0,
            channel: 0,
            rssi_min: 0,
            rssi_max: 0,
            rssi_avg_q8: 0,
            oui_matched: 0,
            signal_kind: 0,
            detection_method: 0,
            ble_company_id: 0,
        },
        mobile_surveillance_last: MobileSurveillanceLastSnapshot {
            last_seen_ms: 0,
            sniffer_id: 0,
            src: "66:77:88:99:AA:BB".to_string(),
            signal_kind: 0,
            detection_method: 0,
        },
        ssid_watch_last: SsidWatchLastSnapshot {
            last_seen_ms: 0,
            sniffer_id: 0,
            bssid: "CC:DD:EE:FF:00:11".to_string(),
            channel: 6,
            rssi: -65,
            ssid: "HiddenAP".to_string(),
        },
        diagnostics: raw(r#"{"heap_free":120000}"#),
    }
}

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("api_stats_golden.json")
}

/// Generate the fixture from the golden body, inserting the single removed key so the
/// golden test can assert it is the *only* difference.
fn generate_fixture_impl() {
    let body = build_golden_body();
    let actual = api_stats_body_to_string(&body);
    let mut val: Value = serde_json::from_str(&actual).expect("body serializes to valid JSON");
    let obj = val.as_object_mut().expect("top-level object");
    obj.insert(REMOVED_KEY.to_string(), Value::from(0));
    let path = fixture_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, serde_json::to_string_pretty(&val).unwrap()).unwrap();
    eprintln!("wrote {}", path.display());
}

#[test]
fn generate_fixture() {
    if std::env::var("GEN_STATS_FIXTURE").is_ok() {
        generate_fixture_impl();
    }
    // Always passes when not generating.
}

/// The single-source golden: the emitted key/value set must equal the fixture minus
/// exactly `detection_wildcard_window_ms`, with every other key and value identical.
#[test]
fn api_stats_body_golden() {
    let body = build_golden_body();
    let actual = api_stats_body_to_string(&body);
    let actual_val: Value = serde_json::from_str(&actual).expect("body serializes to valid JSON");

    let fixture_path = fixture_path();
    assert!(
        fixture_path.exists(),
        "fixture missing — run: GEN_STATS_FIXTURE=1 cargo test-host -p wardriver-stats --test stats_body_golden"
    );
    let fixture = std::fs::read_to_string(&fixture_path).expect("read fixture");
    let fixture_val: Value = serde_json::from_str(&fixture).expect("fixture is valid JSON");

    let actual_obj = actual_val.as_object().expect("body is an object");
    let fixture_obj = fixture_val.as_object().expect("fixture is an object");

    // Expected: fixture keys minus the single removed key.
    let mut expected: std::collections::BTreeMap<&str, Value> = fixture_obj
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    assert!(
        expected.remove(REMOVED_KEY).is_some(),
        "fixture must contain the removed key {REMOVED_KEY}"
    );

    // Same key set.
    let actual_keys: std::collections::BTreeSet<&str> =
        actual_obj.keys().map(|s| s.as_str()).collect();
    let expected_keys: std::collections::BTreeSet<&str> = expected.keys().copied().collect();
    assert_eq!(
        actual_keys, expected_keys,
        "top-level key set differs (expected exactly one removed key: {REMOVED_KEY})"
    );

    // The removed key must NOT be present in the new body.
    assert!(
        !actual_obj.contains_key(REMOVED_KEY),
        "new body must not emit {REMOVED_KEY}"
    );

    // Every key/value identical (top-level + nested objects + array fragments).
    for (k, v) in &expected {
        assert_eq!(actual_obj.get(*k), Some(v), "value mismatch for key {k}");
    }

    // Sanity: exactly 280 top-level keys (old 281 minus 1).
    assert_eq!(actual_obj.len(), 280, "expected 280 top-level keys");
}
