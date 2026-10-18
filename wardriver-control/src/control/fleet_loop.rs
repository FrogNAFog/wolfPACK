//! Scan/log gate evaluation for wolfPACK fleet orchestration.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use esp_idf_sys::wifi_interface_t;
use log::{info, warn};

use crate::config;
use crate::geo;
use crate::gps;
use crate::recon;
use crate::storage;
use crate::time_sync;
use crate::wifi_scan;
use wardriver_stats as stats;

use super::fleet_fsm::FleetOrchestrator;
use super::CONTROL_AP_CHANNEL;

/// Continuous `gps_ok` with `d_m > radius` required before clearing home-zone suppression.
const HOME_ZONE_EXIT_SUSTAINED_MS: u64 = 5_000;
const HOME_ZONE_DIAG_LOG_INTERVAL_MS: u64 = 5_000;

/// Mutable gate/enforce state carried across main-loop iterations.
pub(crate) struct FleetGateState {
    pub inside_home_zone_prev: bool,
    /// `stats::now_ms()` when `gps_ok` and `d_m > radius` began while latched inside; 0 if not accumulating.
    pub home_zone_outside_gps_ok_since_ms: u64,
    pub last_home_zone_log_ms: u64,
    pub gps_ok_prev: bool,
    pub last_schedule_send_ms: u64,
    pub last_ping_time_seq_bump_ms: u64,
    pub log_allowed_prev: bool,
    pub gps_fix_ever_ok: bool,
    pub last_status_cycle_ms: u64,
    pub status_cycle_idx: usize,
}

impl Default for FleetGateState {
    fn default() -> Self {
        Self {
            inside_home_zone_prev: false,
            home_zone_outside_gps_ok_since_ms: 0,
            last_home_zone_log_ms: 0,
            gps_ok_prev: false,
            last_schedule_send_ms: 0,
            last_ping_time_seq_bump_ms: 0,
            log_allowed_prev: false,
            gps_fix_ever_ok: false,
            last_status_cycle_ms: 0,
            status_cycle_idx: 0,
        }
    }
}

/// Read-only inputs for one gate/enforce tick.
pub(crate) struct FleetGateParams<'a> {
    pub alert_hub: &'a Arc<crate::alert_feedback::AlertHub>,
    pub gps_fix: &'a Arc<Mutex<Option<gps::Fix>>>,
    pub home_geo: &'a Arc<Mutex<config::HomeGeoConfig>>,
    pub scanning_allowed: &'a Arc<AtomicBool>,
    pub sniffers: &'a Arc<Mutex<HashSet<[u8; 6]>>>,
    pub cmd_seq: &'a Arc<AtomicU32>,
    pub sd_io: &'a storage::SdIoHandle,
    pub own_management_ap_bssid: Option<[u8; 6]>,
    pub wigle_home: &'a Arc<Mutex<Option<[u8; 6]>>>,
    pub wigle_manual: &'a Arc<RwLock<HashSet<[u8; 6]>>>,
    pub wigle_ssids_manual: &'a Arc<RwLock<HashSet<String>>>,
    pub control_boot_id: u32,
    pub peer_if: wifi_interface_t,
}

/// OLED status-line cycle for one tick: reset the cycle when empty, else rotate on
/// `STATUS_CYCLE_MS` (skipped while an upload is in progress / OLED held).
fn cycle_oled_status(
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
    state: &mut FleetGateState,
    gps_ok: bool,
    fix_snapshot: &Option<gps::Fix>,
) {
    let status_msgs = crate::alert_feedback::oled_status_lines(gps_ok, fix_snapshot.as_ref());
    if status_msgs.is_empty() {
        state.status_cycle_idx = 0;
    } else if !stats::upload_in_progress() && !stats::upload_oled_hold_active() {
        let now_cycle = stats::now_ms();
        if now_cycle.saturating_sub(state.last_status_cycle_ms)
            >= crate::alert_feedback::STATUS_CYCLE_MS
        {
            state.last_status_cycle_ms = now_cycle;
            let idx = state.status_cycle_idx % status_msgs.len();
            let line = &status_msgs[idx];
            alert_hub.show_oled_status(line.title, line.subtitle.as_deref());
            state.status_cycle_idx = state.status_cycle_idx.wrapping_add(1);
            if status_msgs.len() > 1 {
                info!(
                    "alert: OLED status rotate {} ({}/{})",
                    line.title,
                    idx + 1,
                    status_msgs.len()
                );
            }
        }
    }
}

/// Home-geofence latch for one tick: update the inside/outside latch state, write the
/// throttled home-zone diag log line, and return `(outside_home_zone, home_geofence_suppressing)`.
fn latch_home_geofence(
    state: &mut FleetGateState,
    home_geo: &config::HomeGeoConfig,
    fix_snapshot: &Option<gps::Fix>,
    gps_ok: bool,
    now_ms_gate: u64,
    sd_io: &storage::SdIoHandle,
) -> (bool, bool) {
    match config::home_geo_poi_degrees(home_geo) {
        None => {
            state.inside_home_zone_prev = false;
            state.home_zone_outside_gps_ok_since_ms = 0;
            (true, false)
        }
        Some((plat, plon)) => {
            let r = home_geo.radius_m;
            let r_m = f64::from(r);
            let inside = if state.inside_home_zone_prev {
                if gps_ok {
                    if let Some(f) = fix_snapshot.as_ref() {
                        let d_m = geo::haversine_m(f.lat, f.lon, plat, plon);
                        if d_m > r_m {
                            let now_ms_hz = now_ms_gate;
                            if state.home_zone_outside_gps_ok_since_ms == 0 {
                                state.home_zone_outside_gps_ok_since_ms = now_ms_hz;
                            }
                            let sustained_ms =
                                now_ms_hz.saturating_sub(state.home_zone_outside_gps_ok_since_ms);
                            if sustained_ms >= HOME_ZONE_EXIT_SUSTAINED_MS {
                                state.inside_home_zone_prev = false;
                                state.home_zone_outside_gps_ok_since_ms = 0;
                                false
                            } else {
                                true
                            }
                        } else {
                            state.home_zone_outside_gps_ok_since_ms = 0;
                            true
                        }
                    } else {
                        state.home_zone_outside_gps_ok_since_ms = 0;
                        true
                    }
                } else {
                    // Latched inside: keep suppressing until sustained gps_ok outside.
                    state.home_zone_outside_gps_ok_since_ms = 0;
                    true
                }
            } else if gps_ok {
                if let Some(f) = fix_snapshot.as_ref() {
                    let d_m = geo::haversine_m(f.lat, f.lon, plat, plon);
                    state.home_zone_outside_gps_ok_since_ms = 0;
                    if d_m <= r_m {
                        state.inside_home_zone_prev = true;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                // Already outside: stay outside through GPS dropout.
                false
            };

            let now_ms_hz = stats::now_ms();
            if stats::SD_MOUNT_OK.load(Ordering::Acquire)
                && now_ms_hz.saturating_sub(state.last_home_zone_log_ms)
                    >= HOME_ZONE_DIAG_LOG_INTERVAL_MS
            {
                state.last_home_zone_log_ms = now_ms_hz;
                let d_m = fix_snapshot
                    .as_ref()
                    .map(|f| geo::haversine_m(f.lat, f.lon, plat, plon));
                let exit_sustain_ms = if state.inside_home_zone_prev
                    && state.home_zone_outside_gps_ok_since_ms != 0
                {
                    now_ms_hz.saturating_sub(state.home_zone_outside_gps_ok_since_ms)
                } else {
                    0
                };
                let line = format!(
                        "utc={} t_ms={} home_zone=1 gps_ok={} r_m={} d_m={} inside={} exit_sustain_ms={}",
                        time_sync::now_utc_string().unwrap_or_else(|| "-".into()),
                        now_ms_hz,
                        gps_ok as u8,
                        r,
                        wardriver_stats::json_fixed::metres_diag_1dp(d_m),
                        inside as u8,
                        exit_sustain_ms
                    );
                if let Err(e) = storage::append_diag_log_line(sd_io, &line) {
                    warn!("control: home-zone diag log write failed: {e:#}");
                }
            }

            (!inside, inside)
        }
    }
}

/// Evaluate scan/log gates and recon hold.
pub(crate) fn tick_scan_gates_and_enforce(
    state: &mut FleetGateState,
    fleet: &mut FleetOrchestrator,
    params: FleetGateParams<'_>,
) {
    let now_ms_gate = stats::now_ms();
    let gps_nav_fresh = stats::gps_nav_fresh(now_ms_gate);
    let gps_clock_stale = stats::gps_clock_stale(now_ms_gate);
    let fix_snapshot = match params.gps_fix.lock() {
        Ok(mut g) => {
            if !gps_nav_fresh || gps_clock_stale {
                *g = None;
            }
            g.clone()
        }
        Err(_) => None,
    };
    let gps_fix_ok = fix_snapshot
        .as_ref()
        .map(|f| wifi_scan::is_fix_usable_for_location(f))
        .unwrap_or(false);
    let gps_ok = gps_fix_ok && gps_nav_fresh && !gps_clock_stale;
    let home_geo = params
        .home_geo
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    if gps_ok && !state.gps_ok_prev {
        let sub = fix_snapshot
            .as_ref()
            .map(|f| {
                let mut parts = Vec::new();
                if let Some(n) = f.satellites {
                    parts.push(format!("{n} sats"));
                }
                if let Some(h) = f.hdop {
                    parts.push(wardriver_stats::json_fixed::hdop_subtitle_token(h));
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join(" - "))
                }
            })
            .flatten();
        params
            .alert_hub
            .trigger(crate::alert_feedback::AlertKind::GpsFix, sub.as_deref());
    }
    cycle_oled_status(params.alert_hub, state, gps_ok, &fix_snapshot);
    state.gps_ok_prev = gps_ok;
    if gps_ok {
        state.gps_fix_ever_ok = true;
    }

    let (outside_home_zone, home_geofence_suppressing) = latch_home_geofence(
        state,
        &home_geo,
        &fix_snapshot,
        gps_ok,
        now_ms_gate,
        params.sd_io,
    );
    stats::set_home_geofence_suppressing(home_geofence_suppressing);
    // Home geofence: tell sniffers to hold bulk wardrive TX (not scanning).
    stats::set_wardrive_suppressed(home_geofence_suppressing);

    let hold = stats::fleet_management_hold();
    let scan_allowed = !hold && !stats::recon_hold();
    let hist_drain = state.gps_fix_ever_ok && stats::gps_hist_drain_ok();
    let log_allowed = scan_allowed
        && time_sync::wall_clock_trusted()
        && outside_home_zone
        && (gps_ok || hist_drain);
    let prev_scan = params.scanning_allowed.swap(scan_allowed, Ordering::AcqRel);
    stats::set_scan_allowed(scan_allowed);
    stats::set_csv_enqueue_allowed(log_allowed);
    if log_allowed != state.log_allowed_prev {
        if log_allowed {
            let _ = params.sd_io.try_request_open_session();
            let _ = super::fleet_cmd::broadcast_bssid_denylist(
                params.own_management_ap_bssid,
                params.wigle_home,
                params.wigle_manual,
                params.peer_if,
            );
            let _ = super::fleet_cmd::broadcast_ssid_denylist(
                params.wigle_ssids_manual,
                params.peer_if,
            );
            state.last_schedule_send_ms = 0;
        }
    }
    state.log_allowed_prev = log_allowed;
    if scan_allowed != prev_scan {
        if scan_allowed {
            state.last_schedule_send_ms = 0;
        }
    }

    if let Err(e) = recon::try_begin_pending_session(
        params.sd_io,
        params.sniffers,
        params.control_boot_id,
        CONTROL_AP_CHANNEL,
    ) {
        warn!("control: recon begin: {e:#}");
    }
    if stats::recon_hold() {
        params.scanning_allowed.store(false, Ordering::Release);
        stats::set_scan_allowed(false);
        stats::set_csv_enqueue_allowed(false);
    }
    let _ = fleet.sync_from_stats(stats::scan_allowed());
}
