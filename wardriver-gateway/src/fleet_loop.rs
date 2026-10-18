//! Periodic fleet downlink and deferred welcome drain (mirrors control `main_loop_tail`).

use std::sync::atomic::{AtomicU32, Ordering};

use log::{info, warn};
use wardriver_core::liveness::SNIFFER_UPLINK_LIVENESS_MS;
use wardriver_fleet_realtime::fleet_tick::{
    maybe_tick_queue_hint_ping, tick_fleet_ping, FleetTickDeps, FleetTickState,
};
use wardriver_fleet_realtime::gateway_downlink::drain_pending_role_changes as drain_pending_role_jobs;
use wardriver_fleet_realtime::peer_pool;
use wardriver_fleet_realtime::ESPNOW_PEER_IF;
use wardriver_stats as stats;

use crate::adoption_retry;
use crate::channel_plan_downlink;
use crate::policy_downlink::GatewayFleetState;

/// Gateway main-loop fleet tick state (shared with [`FleetTickState`]).
pub type GatewayFleetLoopState = FleetTickState;

fn gateway_fleet_tick_deps(fleet: &GatewayFleetState, broadcast_ready: bool) -> FleetTickDeps<'_> {
    let scan_allowed = fleet.scanning_allowed.load(Ordering::Acquire);
    FleetTickDeps {
        cmd_seq: &fleet.cmd_seq,
        control_boot_id: fleet.control_boot_id.load(Ordering::Relaxed),
        peer_if: ESPNOW_PEER_IF,
        sniffers: &fleet.sniffers,
        scanning_allowed: scan_allowed,
        builder: fleet.cmd_builder(),
        broadcast_ready,
    }
}

pub fn tick(fleet: &GatewayFleetState, state: &mut GatewayFleetLoopState) {
    adoption_retry::tick(fleet);
    drain_pending_role_changes(fleet);

    let now_ms = stats::now_ms();
    let _ = peer_pool::tick_stale_liveness_evict(now_ms);
    log_fleet_liveness_summary(fleet, now_ms);

    // RAM policy mirror only — never gate on softap_transport::phone_connected().
    let broadcast_ready = crate::policy_downlink::fleet_ping_broadcast_ready(
        fleet.policy_applied.load(Ordering::Acquire),
    );
    let deps = gateway_fleet_tick_deps(fleet, broadcast_ready);

    if let Err(e) = maybe_tick_queue_hint_ping(state, &deps, now_ms) {
        warn!("gateway: queue-hint fleet ping: {e:#}");
    }

    if let Err(e) = tick_fleet_ping(state, &deps, now_ms, None) {
        warn!("gateway: periodic fleet ping: {e:#}");
    }

    // Dedicated channel-plan rebalance mirrors control `tick_channel_plan_and_roles`:
    // must not depend on fleet Ping (phone may disconnect while the dedicated plan mirror
    // and roster policy remain active).
    channel_plan_downlink::rebalance_if_dedicated(fleet, Some(stats::time_set_seq_for_wire()));
}

fn drain_pending_role_changes(fleet: &GatewayFleetState) {
    drain_pending_role_jobs(
        &fleet.cmd_seq,
        fleet.control_boot_id.load(Ordering::Relaxed),
        ESPNOW_PEER_IF,
        Some(stats::time_set_seq_for_wire()),
        |sid, role_wire, e| {
            warn!("gateway: SetRole unicast sniffer_id={sid} role={role_wire}: {e:#}");
        },
    );
}

/// 5s gate for the fleet liveness summary (post-eviction state). u32 ms tick
/// (`wrapping_sub` delta); 2^32 ms ≈ 49.7 days between rollovers, far beyond any
/// 5s gate interval.
static LAST_FLEET_LOG_MS: AtomicU32 = AtomicU32::new(0);

/// Periodic `gateway: fleet alive=N/M quiet=[...]` serial line (post-eviction).
/// RAM policy mirror only — never gate on softap_transport::phone_connected();
/// the fleet path stays active regardless of phone state.
fn log_fleet_liveness_summary(fleet: &GatewayFleetState, now_ms: u64) {
    let now_u32 = now_ms as u32;
    if now_u32.wrapping_sub(LAST_FLEET_LOG_MS.load(Ordering::Relaxed)) < 5000 {
        return;
    }
    LAST_FLEET_LOG_MS.store(now_u32, Ordering::Relaxed);

    let Some(macs) = fleet
        .sniffers
        .lock()
        .ok()
        .map(|g| g.iter().copied().collect::<Vec<_>>())
    else {
        return;
    };
    let roster = macs.len() as u32;
    if roster == 0 {
        info!("gateway: fleet alive=0/0 (no roster)");
        return;
    }
    let mut quiet: Vec<(u32, u64)> = Vec::new();
    let mut alive = 0u32;
    for mac in macs {
        let sid = stats::sniffer_id_for_mac(&mac).unwrap_or(0);
        let last_seen = stats::sniffer_last_seen_ms(sid).unwrap_or(0);
        let age_ms = now_ms.saturating_sub(last_seen);
        if last_seen > 0 && age_ms <= SNIFFER_UPLINK_LIVENESS_MS {
            alive += 1;
        } else {
            quiet.push((sid, age_ms));
        }
    }
    quiet.sort_by_key(|(sid, _)| *sid);
    let quiet_items = quiet
        .iter()
        .map(|(sid, age_ms)| format!("{sid}:{age_ms}ms"))
        .collect::<Vec<_>>()
        .join(",");
    info!("gateway: fleet alive={alive}/{roster} quiet=[{quiet_items}]");
}
