//! Shared periodic fleet `Ping` tick for control and gateway main loops.
//!
//! Host-testable gating helpers live at module root; ESP-NOW broadcast paths require `esp`.

use crate::fleet_commands::{periodic_ping_due, queue_hint_ping_should_send};

/// Mutable state for periodic ping cadence and piggybacked `time_set_seq` bumps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FleetTickState {
    pub last_periodic_ping_ms: u64,
    pub last_ping_time_seq_bump_ms: u64,
}

/// True when the gateway should broadcast wolfPACK fleet `Ping` (peer join hints).
///
/// Gateway passes `policy_applied` from the RAM policy mirror; PackLink TCP is not
/// required and must never gate this helper.
#[must_use]
pub fn fleet_ping_broadcast_ready(policy_applied: bool) -> bool {
    policy_applied
}

/// Returns whether a periodic fleet ping should run this iteration.
///
/// Gateway passes [`fleet_ping_broadcast_ready`] (`policy_applied` only); control always
/// passes `true` on the scanning path.
#[must_use]
pub fn periodic_tick_should_run(broadcast_ready: bool, last_ping_ms: u64, now_ms: u64) -> bool {
    broadcast_ready && periodic_ping_due(last_ping_ms, now_ms)
}

/// Returns whether a queue-hint ping should run.
///
/// `queue_hint_due` is the result of [`crate::peer_pool::take_queue_hint_ping_due`] on ESP builds.
#[must_use]
pub fn queue_hint_tick_should_run(
    broadcast_ready: bool,
    queue_hint_due: bool,
    last_ping_ms: u64,
    now_ms: u64,
) -> bool {
    broadcast_ready && queue_hint_due && queue_hint_ping_should_send(last_ping_ms, now_ms)
}

#[cfg(feature = "esp")]
mod esp {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    use log::info;

    use anyhow::Result;
    use esp_idf_sys::wifi_interface_t;
    use wardriver_proto::ControlCmd;

    use super::{periodic_tick_should_run, queue_hint_tick_should_run, FleetTickState};
    use crate::gateway_downlink::{broadcast_cmd, bump_time_set_seq_if_due, FleetCommandBuilder};
    use crate::peer_pool;

    /// 5 s gate for the fleet_ping tx diagnostic log (u32 ms, ~49.7 d rollover).
    static LAST_PING_LOG_MS: AtomicU32 = AtomicU32::new(0);

    /// Per-tick inputs shared by control and gateway fleet ping paths.
    pub struct FleetTickDeps<'a> {
        pub cmd_seq: &'a AtomicU32,
        pub control_boot_id: u32,
        pub peer_if: wifi_interface_t,
        pub sniffers: &'a Arc<Mutex<HashSet<[u8; 6]>>>,
        /// When true, sniffers should scan; drives `desired_scanning` on the Ping frame.
        pub scanning_allowed: bool,
        pub builder: FleetCommandBuilder,
        /// When false, skip all broadcast ping work (gateway: policy not applied).
        /// Never tied to PackLink phone_connected.
        pub broadcast_ready: bool,
    }

    /// Broadcast fleet `Ping` and advance [`FleetTickState`] timestamps.
    fn broadcast_fleet_ping(
        state: &mut FleetTickState,
        deps: &FleetTickDeps<'_>,
        now_ms: u64,
    ) -> Result<()> {
        state.last_periodic_ping_ms = now_ms;
        let gps_time_seq = bump_time_set_seq_if_due(&mut state.last_ping_time_seq_bump_ms, now_ms);
        let desired_scanning = u8::from(deps.scanning_allowed);
        // 5 s-cadence diagnostic: is the gateway actually emitting fleet pings
        // (with peers) independent of phone state?
        if now_ms.saturating_sub(u64::from(LAST_PING_LOG_MS.load(Ordering::Relaxed))) >= 5000 {
            LAST_PING_LOG_MS.store((now_ms % u64::from(u32::MAX)) as u32, Ordering::Relaxed);
            let peers = deps.sniffers.lock().unwrap().len();
            info!(
                "fleet_ping: tx peers={} boot_id={} cmd_seq={}",
                peers,
                deps.control_boot_id,
                deps.cmd_seq.load(Ordering::Relaxed),
            );
        }
        broadcast_cmd(
            deps.sniffers,
            deps.cmd_seq,
            ControlCmd::Ping,
            None,
            deps.peer_if,
            deps.control_boot_id,
            true,
            &deps.builder,
            gps_time_seq,
            desired_scanning,
        )
    }

    /// Periodic ~300 ms fleet `Ping` when [`periodic_tick_should_run`] is true.
    ///
    /// `after_periodic_ping` runs after the ping attempt when the periodic interval elapsed
    /// (gateway: dedicated channel-plan rebalance), regardless of ESP-NOW send success.
    pub fn tick_fleet_ping(
        state: &mut FleetTickState,
        deps: &FleetTickDeps<'_>,
        now_ms: u64,
        after_periodic_ping: Option<&mut dyn FnMut()>,
    ) -> Result<()> {
        if !periodic_tick_should_run(deps.broadcast_ready, state.last_periodic_ping_ms, now_ms) {
            return Ok(());
        }
        let result = broadcast_fleet_ping(state, deps, now_ms);
        if let Some(cb) = after_periodic_ping {
            cb();
        }
        result
    }

    /// Extra `Ping` when the peer join queue changed so waiters wake on fresh hints.
    pub fn maybe_tick_queue_hint_ping(
        state: &mut FleetTickState,
        deps: &FleetTickDeps<'_>,
        now_ms: u64,
    ) -> Result<()> {
        let queue_hint_due = peer_pool::take_queue_hint_ping_due(now_ms);
        if !queue_hint_tick_should_run(
            deps.broadcast_ready,
            queue_hint_due,
            state.last_periodic_ping_ms,
            now_ms,
        ) {
            return Ok(());
        }
        broadcast_fleet_ping(state, deps, now_ms)
    }
}

#[cfg(feature = "esp")]
pub use esp::{maybe_tick_queue_hint_ping, tick_fleet_ping, FleetTickDeps};
