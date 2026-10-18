//! `/api/stats` body: control-side adapter.
//!
//! The body itself is a single serde-driven [`wardriver_stats::ApiStatsBody`] — the
//! single source of truth for the endpoint. This module gathers the control-only
//! inputs (home-geo config, GPS fix, wall-clock trust, peer-pool gauges, diagnostics
//! object, recon path, forms rev) and hands them to [`wardriver_stats::build_api_stats_body`].

use std::sync::{Arc, Mutex};

use crate::config;
use crate::control::fleet_fsm::FleetPhase;
use crate::control::peer_pool;
use crate::gps;
use crate::recon;
use crate::time_sync;
use wardriver_board;
use wardriver_stats as stats;
use wardriver_stats::{json_fixed, GpsFixInput};

use super::pages::describe_gps_for_home_zone;
use super::stats_json_diag;

pub(crate) fn build_api_stats_json(
    wigle_api_name: &str,
    home_geo_stats: &Arc<Mutex<config::HomeGeoConfig>>,
    gps_stats: &Arc<Mutex<Option<gps::Fix>>>,
) -> String {
    // Same `HomeGeoConfig` the control loop uses — not a fresh NVS read — so this cannot
    // disagree with `home_geofence_suppressing` (which the loop sets from this mutex).
    let home_cfg = home_geo_stats
        .lock()
        .ok()
        .map(|c| c.clone())
        .unwrap_or_default();
    let home_geo_configured = home_cfg.poi.is_some() as u8;

    let home_zone_poi_micro = config::home_geo_poi_degrees(&home_cfg).map(|(lat, lon)| {
        (
            json_fixed::degrees_to_microdeg(lat),
            json_fixed::degrees_to_microdeg(lon),
        )
    });

    let fix_snapshot = gps_stats.lock().ok().and_then(|g| g.clone());
    let gps_fix = fix_snapshot.as_ref().map(|f| GpsFixInput {
        fix_quality: f.fix_quality,
        hdop: f.hdop,
        satellites: f.satellites,
    });

    let body = stats::build_api_stats_body(&stats::StatsBodyInputs {
        wigle_api_name,
        home_zone_poi_micro,
        home_zone_radius_m: home_cfg.radius_m,
        home_zone_configured: home_geo_configured,
        home_zone_gps_line: &describe_gps_for_home_zone(gps_stats),
        gps_fix,
        wall_clock_trusted: time_sync::wall_clock_trusted(),
        fleet_phase_name: FleetPhase::from_u8(stats::fleet_phase()).name(),
        recon_output_path: &recon::output_relpath_for_api(),
        board_profile: wardriver_board::active_profile().name,
        firmware_version: env!("CARGO_PKG_VERSION"),
        esp_idf_version: option_env!("ESP_IDF_VERSION").unwrap_or("unknown"),
        management_ap_ssid: &management_ap_ssid(),
        peer_pool_active: peer_pool::active_count().min(255) as u8,
        peer_slots_free: peer_pool::peer_slots_free_estimate(),
        peer_join_waiters: peer_pool::join_queue_waiter_count(),
        diagnostics_json: &stats_json_diag::build_diagnostics_json(),
        forms_rev: super::snapshots::forms_rev(),
        control_cpu_load_pct: wardriver_shared::cpu_load::pct(),
        reset_reason_str: wardriver_shared::boot_diag::reset_reason_str(),
        reset_reason_raw: wardriver_shared::boot_diag::reset_reason_raw(),
        soc_temp_c_x10: wardriver_shared::soc_temp::read_celsius()
            .map(|t| json_fixed::f32_to_x10(t)),
    });

    stats::api_stats_body_to_string(&body)
}

fn management_ap_ssid() -> String {
    let mas_stored = stats::management_ap_ssid_display();
    if mas_stored.is_empty() {
        wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_SSID.to_string()
    } else {
        mas_stored
    }
}
