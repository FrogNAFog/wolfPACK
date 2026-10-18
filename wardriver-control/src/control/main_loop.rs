//! One iteration of the control-node main loop (fleet gates, GPS, uploads).
//!
//! `control_loop_tick` runs a fixed sequence of sub-ticks in one file so the tick order is
//! visible end-to-end. Each sub-tick takes only the `ControlLoopDeps` sub-struct(s) it uses
//! (see [`super::main_loop_state`]) so signatures declare actual dependency:
//! prelude (fleet+exclusion+core) → web snapshots → core (all four) → nvs drain → tail (fleet+core).

pub(crate) use super::main_loop_state::{
    ControlLoopDeps, ControlLoopState, CoreDeps, ExclusionDeps, FleetDeps, GpsDeps,
    GpsRecoveryWindow,
};

use log::{info, warn};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::web::snapshots::{web_snapshots_tick_try, WebSnapshotInputs};
use wardriver_shared::boot_button;
use wardriver_stats as stats;

use super::csv_ingest::{gps_hist_can_drain, unix_now_ms};
use super::exclusion::try_gps_soft_or_hard_recovery;
use super::fleet_cmd;
use super::fleet_fsm::apply_dashboard_mode_requests;
use super::fleet_loop::{tick_scan_gates_and_enforce, FleetGateParams};
use super::fleet_state::fill_active_wifi_sniffers;
use super::nvs_defer_queue::{drain_nvs_defer_queue, take_pending_provisioning_reboot};
use crate::channel_plan;
use crate::storage;
use crate::time_sync;
use wardriver_fleet_realtime::channel_plan_runtime::hash_u32_list;
use wardriver_fleet_realtime::fleet_tick::{tick_fleet_ping, FleetTickDeps, FleetTickState};
use wardriver_proto as proto;
use wardriver_shared::cpu_load;

const GPS_RECOVERY_CHECK_INTERVAL_MS: u64 = 5000;
const GPS_SOFT_RECOVERY_NAV_MS: u64 = 8000;
const GPS_SOFT_RECOVERY_CLOCK_MS: u64 = stats::GPS_CLOCK_STALE_MS;
const GPS_SOFT_RECOVERY_UART_IDLE_MS: u64 = 5000;
const GPS_HARD_RECOVERY_UART_IDLE_MS: u64 = 30_000;
const GPS_SOFT_RECOVERY_MAX_PER_WINDOW: u32 = 3;
const GPS_SOFT_RECOVERY_WINDOW_MS: u64 = 300_000;

const DROP_GRACE_MS: u64 = 10_000;
const SCHEDULE_RESEND_MIN_MS: u64 = 1_000;

const CONTROL_LOOP_BUDGET_MS: u64 = 20;
/// Optional main-loop heartbeat line when diag logging is enabled.
const LOOP_HEARTBEAT_INTERVAL_MS: u64 = 5_000;

pub(crate) fn control_loop_tick(state: &mut ControlLoopState, deps: &mut ControlLoopDeps<'_>) {
    let iter_start = stats::now_ms();
    // Feed TWDT at iteration start so a stall in prelude/core still trips within timeout.
    wardriver_shared::task_wdt::reset_if_subscribed();
    stats::set_control_main_loop_tick_ms(iter_start);

    stats::set_control_main_loop_phase(stats::CONTROL_LOOP_PHASE_PRELUDE);
    tick_loop_prelude(state, &mut deps.fleet, &mut deps.exclusion, &mut deps.core);

    wardriver_shared::task_wdt::reset_if_subscribed();
    let now_snap = stats::now_ms();
    let snapshot_inputs = WebSnapshotInputs {
        nvs: deps.core.nvs,
        home_geo: &deps.exclusion.home_geo,
        gps_fix: &deps.gps.gps_fix,
        exclude_manual: &deps.exclusion.wigle_manual,
        exclude_ssids_manual: &deps.exclusion.wigle_ssids_manual,
        detection_exclude_manual: &deps.exclusion.detection_ignore,
        ssid_watchlist: &deps.exclusion.ssid_watchlist,
    };
    web_snapshots_tick_try(&deps.core.web_snapshots, &snapshot_inputs, now_snap);

    stats::set_control_main_loop_phase(stats::CONTROL_LOOP_PHASE_CORE);
    tick_loop_core(
        state,
        &mut deps.fleet,
        &mut deps.exclusion,
        &mut deps.gps,
        &mut deps.core,
    );

    wardriver_shared::task_wdt::reset_if_subscribed();
    drain_nvs_defer(&mut deps.core, &mut deps.exclusion);
    stats::set_control_main_loop_phase(stats::CONTROL_LOOP_PHASE_TAIL);
    tick_loop_tail(state, &mut deps.fleet, &mut deps.core);

    let iter_ms = stats::now_ms().saturating_sub(iter_start);
    stats::set_control_main_loop_last_iter_ms(iter_ms);
}

// ---------------------------------------------------------------------------
// Prelude: denylist rebroadcast, dashboard mode (fleet FSM inputs), fleet unbind.
// ---------------------------------------------------------------------------

fn drain_nvs_defer(core: &mut CoreDeps<'_>, exclusion: &mut ExclusionDeps) {
    drain_nvs_defer_queue(
        &core.nvs_defer_rx,
        core.nvs,
        &exclusion.home_geo,
        &exclusion.wigle_home,
    );
}

fn tick_loop_prelude(
    state: &mut ControlLoopState,
    fleet: &mut FleetDeps,
    exclusion: &mut ExclusionDeps,
    core: &mut CoreDeps<'_>,
) {
    if take_pending_provisioning_reboot() {
        drain_nvs_defer(core, exclusion);
        let _ = core.sd_io.rpc_count_pending_csv();
        std::thread::sleep(Duration::from_millis(100));
        unsafe { esp_idf_sys::esp_restart() };
    }
    drain_nvs_defer(core, exclusion);
    crate::storage::run_deferred_boot_wigle_cleanup(core.sd_io.as_ref());
    drain_pending_deferred_welcomes(state, fleet, exclusion);

    if exclusion.denylist_dirty.swap(false, Ordering::AcqRel) {
        if let Err(e) = fleet_cmd::broadcast_bssid_denylist(
            fleet.own_management_ap_bssid,
            &exclusion.wigle_home,
            &exclusion.wigle_manual,
            fleet.peer_if,
        ) {
            warn!("control: BSSID denylist rebroadcast: {e:#}");
        }
        if let Err(e) =
            fleet_cmd::broadcast_ssid_denylist(&exclusion.wigle_ssids_manual, fleet.peer_if)
        {
            warn!("control: SSID denylist rebroadcast: {e:#}");
        }
    }

    if exclusion.watchlist_dirty.swap(false, Ordering::AcqRel) {
        if let Err(e) =
            fleet_cmd::broadcast_ssid_watchlist(&exclusion.ssid_watchlist, fleet.peer_if)
        {
            warn!("control: SSID watchlist rebroadcast: {e:#}");
        }
    }

    if exclusion
        .detection_masks_dirty
        .swap(false, Ordering::AcqRel)
    {
        if let Err(e) = fleet_cmd::broadcast_detection_disable_wifi_mask(
            &fleet.sniffers,
            &fleet.cmd_seq,
            fleet.peer_if,
            fleet.control_boot_id,
        ) {
            warn!("control: detection_disable_wifi_mask rebroadcast: {e:#}");
        }
    }

    apply_dashboard_mode_requests(fleet, core);
    tick_boot_fleet_unbind(state, fleet, core);
}

fn tick_boot_fleet_unbind(
    state: &mut ControlLoopState,
    fleet: &mut FleetDeps,
    core: &mut CoreDeps<'_>,
) {
    let hold_ms = boot_button::hold_elapsed_ms();
    if hold_ms > 0 {
        #[cfg(feature = "board-xiao")]
        if let Some(label) = boot_button::format_unbind_countdown(hold_ms) {
            let sec = hold_ms.div_ceil(1000);
            if state.boot_unbind_last_oled_sec != Some(sec) {
                state.boot_unbind_last_oled_sec = Some(sec);
                core.alert_hub.show_oled_status("UNBIND", Some(&label));
            }
        }
        #[cfg(not(feature = "board-xiao"))]
        boot_button::log_hold_progress_if_due(&mut state.boot_unbind_last_oled_sec, hold_ms);
    } else {
        state.boot_unbind_last_oled_sec = None;
    }

    if boot_button::take_fleet_unbind_request() {
        match fleet_cmd::unbind_all_fleet(core.nvs, fleet.peer_if) {
            Ok(()) => core
                .alert_hub
                .trigger(stats::alert_kinds::AlertKind::FleetUnbound, None),
            Err(e) => warn!("control: fleet unbind: {e:#}"),
        }
    }
}

fn drain_pending_deferred_welcomes(
    _state: &mut ControlLoopState,
    fleet: &mut FleetDeps,
    exclusion: &mut ExclusionDeps,
) {
    let time_set_seq = stats::time_set_seq_for_wire();
    wardriver_fleet_realtime::gateway_downlink::drain_deferred_welcomes(
        fleet.peer_if,
        "deferred welcome espnow peer sniffer",
        |job| match fleet_cmd::send_sniffer_hello_welcome(
            job.mac,
            job.sniffer_id,
            job.fleet_slot,
            fleet.control_boot_id,
            &fleet.cmd_seq,
            time_set_seq,
            fleet.own_management_ap_bssid,
            &exclusion.wigle_home,
            &exclusion.wigle_manual,
            &exclusion.wigle_ssids_manual,
            &exclusion.ssid_watchlist,
            fleet.peer_if,
        ) {
            Ok(()) => true,
            Err(e) => {
                warn!(
                    "control: deferred welcome sniffer_id={}: {e:#}",
                    job.sniffer_id
                );
                false
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Core: fleet gates, channel plan + roles, GPS recovery, peer-pool maintenance.
// ---------------------------------------------------------------------------

fn tick_loop_core(
    state: &mut ControlLoopState,
    fleet: &mut FleetDeps,
    exclusion: &mut ExclusionDeps,
    gps: &mut GpsDeps,
    core: &mut CoreDeps<'_>,
) {
    tick_scan_gates_and_enforce(
        &mut state.fleet_gate,
        &mut state.fleet,
        FleetGateParams {
            alert_hub: &core.alert_hub,
            gps_fix: &gps.gps_fix,
            home_geo: &exclusion.home_geo,
            scanning_allowed: &fleet.scanning_allowed,
            sniffers: &fleet.sniffers,
            cmd_seq: &fleet.cmd_seq,
            sd_io: core.sd_io.as_ref(),
            own_management_ap_bssid: fleet.own_management_ap_bssid,
            wigle_home: &exclusion.wigle_home,
            wigle_manual: &exclusion.wigle_manual,
            wigle_ssids_manual: &exclusion.wigle_ssids_manual,
            control_boot_id: fleet.control_boot_id,
            peer_if: fleet.peer_if,
        },
    );
    tick_channel_plan_and_roles(state, fleet);
    tick_gps_and_timeset(state, gps);
    {
        let now_ms = stats::now_ms();
        let _ = super::peer_pool::tick_visit_arm_expiry(now_ms);
        let _ = super::peer_pool::tick_join_queue_prune(now_ms);
        let _ = super::peer_pool::tick_stale_liveness_evict(now_ms);
        let now_ms = stats::now_ms();
        let tick_deps = control_fleet_tick_deps(fleet);
        let mut tick_state = control_fleet_tick_state(state);
        let _ = wardriver_fleet_realtime::fleet_tick::maybe_tick_queue_hint_ping(
            &mut tick_state,
            &tick_deps,
            now_ms,
        );
        apply_control_fleet_tick_state(state, &tick_state);
    }
}

// ---------------------------------------------------------------------------
// Schedule: operator role changes and channel-plan push to live Wi-Fi sniffers.
// ---------------------------------------------------------------------------

fn tick_channel_plan_and_roles(state: &mut ControlLoopState, fleet: &mut FleetDeps) {
    use std::collections::HashMap;
    let scan_allowed = stats::scan_allowed();
    let now_ms = stats::now_ms();
    let gps_time_seq = Some(stats::time_set_seq_for_wire());
    // First-send SetRole for operator role changes (ACK-driven resend handled in ingress).
    wardriver_fleet_realtime::gateway_downlink::drain_pending_role_changes(
        &fleet.cmd_seq,
        fleet.control_boot_id,
        fleet.peer_if,
        gps_time_seq,
        |_sid, _role_wire, e| {
            warn!("control: SetRole unicast: {e:#}");
        },
    );

    // 1.5) Apply operator-edited channel plan to the live sniffer set.
    if scan_allowed {
        fill_active_wifi_sniffers(&mut state.active_wifi_buf, now_ms);
        let active = &state.active_wifi_buf;
        state.channel_plan_sig.clear();
        state
            .channel_plan_sig
            .extend(active.iter().map(|(id, _)| *id));
        state.channel_plan_sig.sort_unstable();
        state.channel_plan_sig.dedup();
        let sig_vec = &state.channel_plan_sig;

        let active_sig = hash_u32_list(sig_vec);
        let active_changed = active_sig != state.last_active_sig;
        let plan_rev = channel_plan::revision();
        let plan_changed = plan_rev != state.last_plan_revision;
        let resend_due =
            now_ms.saturating_sub(state.fleet_gate.last_schedule_send_ms) >= SCHEDULE_RESEND_MIN_MS;
        let rebalance_ok = now_ms.saturating_sub(state.last_rebalance_ms) >= DROP_GRACE_MS;

        if resend_due && (plan_changed || (active_changed && rebalance_ok)) {
            let cached = super::channel_plan_cache::get_or_build(sig_vec);

            state.last_plan_revision = plan_rev;
            state.last_active_sig = active_sig;
            state.fleet_gate.last_schedule_send_ms = now_ms;
            state.last_rebalance_ms = now_ms;

            let mut id_to_mac: HashMap<u32, [u8; 6]> = HashMap::new();
            for (id, mac) in active.iter().copied() {
                id_to_mac.insert(id, mac);
            }

            for (sniffer_id, sched) in &cached.schedules {
                let band_wire = sched.band.map(|b| b.as_wire()).unwrap_or(0);
                let pinned_wire = if sched.pinned { sched.channels[0] } else { 0 };
                stats::set_sniffer_plan_assignment(
                    *sniffer_id,
                    band_wire,
                    pinned_wire,
                    sched.chan_count,
                    &sched.channels,
                );

                let Some(mac) = id_to_mac.get(sniffer_id).copied() else {
                    continue;
                };
                fleet_cmd::send_set_channel_schedule_unicast(
                    mac,
                    *sniffer_id,
                    sched,
                    &fleet.cmd_seq,
                    fleet.control_boot_id,
                    fleet.peer_if,
                    gps_time_seq,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GPS: recovery and rolling fix history for the control main loop.
// ---------------------------------------------------------------------------

/// GPS freshness inputs for one recovery-check tick.
pub(crate) struct GpsRecoveryAges {
    pub rmc_age: u64,
    pub gga_age: u64,
    pub uart_idle: u64,
}

/// What [`GpsRecoveryPolicy::tick`] decided. The caller performs the supervisor call.
pub(crate) enum GpsRecoveryAction {
    /// Nothing to do this tick.
    None,
    /// Run the shared soft/hard recovery helper (handles its own window bookkeeping).
    TrySoftOrHard,
    /// Bump the window counter and request a soft recovery (no restart).
    SoftRecovery,
    /// Force a full reader restart (window reset handled by the policy).
    HardRestart,
}

/// GPS soft/hard-recovery decision policy. Owns all window-state math so the caller only
/// performs the supervisor calls. The GPS history drain is a separate concern and is not part
/// of this policy.
pub(crate) struct GpsRecoveryPolicy;

impl GpsRecoveryPolicy {
    pub(crate) fn tick(
        now_ms: u64,
        ages: GpsRecoveryAges,
        window: &mut GpsRecoveryWindow,
    ) -> GpsRecoveryAction {
        let GpsRecoveryAges {
            rmc_age,
            gga_age,
            uart_idle,
        } = ages;

        if stats::gps_clock_stale(now_ms) && rmc_age > GPS_SOFT_RECOVERY_CLOCK_MS {
            return GpsRecoveryAction::TrySoftOrHard;
        }
        if !stats::gps_nav_fresh(now_ms)
            && rmc_age > GPS_SOFT_RECOVERY_NAV_MS
            && gga_age > GPS_SOFT_RECOVERY_NAV_MS
        {
            return GpsRecoveryAction::TrySoftOrHard;
        }
        if stats::gps_nav_fresh(now_ms) && !stats::gps_clock_stale(now_ms) {
            window.in_window = 0;
        }

        if !stats::gps_uart_fresh(now_ms) && uart_idle > GPS_SOFT_RECOVERY_UART_IDLE_MS {
            if window.window_start_ms == 0
                || now_ms.saturating_sub(window.window_start_ms) > GPS_SOFT_RECOVERY_WINDOW_MS
            {
                window.in_window = 0;
                window.window_start_ms = now_ms;
            }
            if window.in_window < GPS_SOFT_RECOVERY_MAX_PER_WINDOW {
                window.in_window = window.in_window.saturating_add(1);
                return GpsRecoveryAction::SoftRecovery;
            }
            if uart_idle > GPS_HARD_RECOVERY_UART_IDLE_MS
                || window.in_window >= GPS_SOFT_RECOVERY_MAX_PER_WINDOW
            {
                window.in_window = 0;
                window.window_start_ms = now_ms;
                return GpsRecoveryAction::HardRestart;
            }
        }

        GpsRecoveryAction::None
    }
}

fn tick_gps_and_timeset(state: &mut ControlLoopState, gps: &mut GpsDeps) {
    let now_ms_ts = stats::now_ms();

    stats::gps_rate_tick(now_ms_ts);

    if now_ms_ts.saturating_sub(state.last_gps_recovery_check_ms) >= GPS_RECOVERY_CHECK_INTERVAL_MS
    {
        state.last_gps_recovery_check_ms = now_ms_ts;
        stats::gps_freshness_tick(now_ms_ts);
        if state.fleet_gate.gps_fix_ever_ok {
            let rmc_age = stats::gps_rmc_last_parse_age_ms(now_ms_ts);
            let gga_age = stats::gps_gga_last_parse_age_ms(now_ms_ts);
            let uart_idle = stats::gps_uart_rx_age_ms(now_ms_ts);
            let action = GpsRecoveryPolicy::tick(
                now_ms_ts,
                GpsRecoveryAges {
                    rmc_age,
                    gga_age,
                    uart_idle,
                },
                &mut state.gps_recovery,
            );
            match action {
                GpsRecoveryAction::None => {}
                GpsRecoveryAction::TrySoftOrHard => try_gps_soft_or_hard_recovery(
                    &gps.gps_supervisor,
                    rmc_age,
                    &mut state.gps_recovery.in_window,
                    &mut state.gps_recovery.window_start_ms,
                    now_ms_ts,
                ),
                GpsRecoveryAction::SoftRecovery => gps.gps_supervisor.request_soft_recovery(),
                GpsRecoveryAction::HardRestart => {
                    if let Err(e) = gps.gps_supervisor.restart_reader() {
                        warn!("control: gps hard restart: {e:#}");
                    }
                }
            }
        }
    }

    let now_unix_ms = unix_now_ms().map(|v| v as i64).unwrap_or(0);
    let threshold = now_unix_ms.saturating_sub(proto::GPS_HIST_RETENTION_MS as i64);
    if let Ok(mut hist) = state.gps_hist.write() {
        while let Some((t, _)) = hist.front() {
            if now_unix_ms != 0 && *t < threshold {
                let _ = hist.pop_front();
            } else {
                break;
            }
        }
        stats::set_gps_hist_drain_ok(gps_hist_can_drain(&hist, now_unix_ms));
    } else {
        stats::set_gps_hist_drain_ok(false);
    }
}

// ---------------------------------------------------------------------------
// Tail: periodic fleet ping, SD diagnostics, loop budget sleep.
// ---------------------------------------------------------------------------

fn control_fleet_tick_state(state: &mut ControlLoopState) -> FleetTickState {
    FleetTickState {
        last_periodic_ping_ms: state.last_periodic_ping_ms,
        last_ping_time_seq_bump_ms: state.fleet_gate.last_ping_time_seq_bump_ms,
    }
}

fn apply_control_fleet_tick_state(state: &mut ControlLoopState, tick: &FleetTickState) {
    state.last_periodic_ping_ms = tick.last_periodic_ping_ms;
    state.fleet_gate.last_ping_time_seq_bump_ms = tick.last_ping_time_seq_bump_ms;
}

fn control_fleet_tick_deps(fleet: &FleetDeps) -> FleetTickDeps<'_> {
    let scan_allowed = fleet.scanning_allowed.load(Ordering::Acquire);
    FleetTickDeps {
        cmd_seq: fleet.cmd_seq.as_ref(),
        control_boot_id: fleet.control_boot_id,
        peer_if: fleet.peer_if,
        sniffers: &fleet.sniffers,
        scanning_allowed: scan_allowed,
        builder: super::fleet_cmd::fleet_cmd_builder_from_plan(),
        broadcast_ready: true,
    }
}

fn tick_loop_tail(state: &mut ControlLoopState, fleet: &mut FleetDeps, core: &mut CoreDeps<'_>) {
    // ~1 Hz FreeRTOS busy % for dashboard `control_cpu_load_pct` (not tied to ESP-NOW RX).
    cpu_load::poll(stats::now_ms());

    // Periodic Ping: link keepalive; also carries wardrive intent on wolfPACK.
    let now_ms_ping = stats::now_ms();
    let tick_deps = control_fleet_tick_deps(fleet);
    let mut tick_state = control_fleet_tick_state(state);
    let _ = tick_fleet_ping(&mut tick_state, &tick_deps, now_ms_ping, None);
    apply_control_fleet_tick_state(state, &tick_state);

    // SD diagnostics for post-drive analysis (heap + dedupe size; first at ~10s, then every 60s).
    let now_loop = stats::now_ms();
    stats::refresh_memory_low_watermarks();
    let diag_due = (state.last_diag_log_ms == 0 && now_loop >= 10_000)
        || (state.last_diag_log_ms > 0
            && now_loop.saturating_sub(state.last_diag_log_ms) >= 60_000);
    let heartbeat_due = stats::diag_log_enabled()
        && (state.last_loop_heartbeat_ms == 0
            || now_loop.saturating_sub(state.last_loop_heartbeat_ms) >= LOOP_HEARTBEAT_INTERVAL_MS);
    if heartbeat_due {
        state.last_loop_heartbeat_ms = now_loop;
        let line = format!(
            "LOOP_HEARTBEAT phase={} tick_ms={} snapshot_age_ms={} iter_ms={} snap_skip={}",
            stats::control_main_loop_phase(),
            stats::control_main_loop_tick_ms(),
            stats::control_snapshot_age_ms(now_loop),
            stats::control_main_loop_last_iter_ms(),
            stats::control_snapshot_write_skipped(),
        );
        let _ = storage::append_diag_log_line(core.sd_io.as_ref(), &line);
    }
    if diag_due {
        state.last_diag_log_ms = now_loop;
        if stats::SD_MOUNT_OK.load(Ordering::Acquire) {
            let uptime = now_loop.saturating_sub(stats::boot_ms());
            let utc = time_sync::now_utc_string().unwrap_or_else(|| "-".into());
            let (mac_table_entries, mac_table_geo_active) = (
                stats::MAC_TABLE_ENTRIES.load(Ordering::Relaxed) as usize,
                stats::MAC_TABLE_GEO_ACTIVE.load(Ordering::Relaxed) as usize,
            );
            let line = format!(
                "utc={} t_ms={} uptime_ms={} rst={} rst_n={} free_heap={} min_ever_free_heap={} free_internal_heap={} free_spiram_heap={} mac_table_entries={} mac_table_geo_active={} unique_wifi={} unique_ble={} dedup_evict={} wifi_rows={}",
                utc,
                now_loop,
                uptime,
                wardriver_shared::boot_diag::reset_reason_str(),
                wardriver_shared::boot_diag::reset_reason_raw(),
                stats::free_heap_bytes(),
                stats::min_ever_free_heap_bytes(),
                stats::free_internal_heap_bytes(),
                stats::free_spiram_heap_bytes(),
                mac_table_entries,
                mac_table_geo_active,
                stats::unique_wifi_aps(),
                stats::unique_ble_devices(),
                stats::geodeduper_evictions(),
                stats::wifi_rows_written()
            );
            if let Err(e) = storage::append_diag_log_line(core.sd_io.as_ref(), &line) {
                warn!("control: diag log to SD: {e:#}");
            } else {
                info!("wolfpack/diag: {line}");
            }
        }
    }

    let loop_elapsed = stats::now_ms().saturating_sub(now_loop);
    if loop_elapsed > CONTROL_LOOP_BUDGET_MS {
        stats::CONTROL_LOOP_OVERRUNS.fetch_add(1, Ordering::Relaxed);
    }
    wardriver_shared::task_wdt::reset_if_subscribed();
    std::thread::sleep(Duration::from_millis(50));
}
