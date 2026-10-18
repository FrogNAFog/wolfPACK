//! Gateway adoption retry tick: one FSM-aware welcome queue (deferred + timed retries).

use crate::gateway_adoption;
use log::warn;
use wardriver_fleet_realtime::gateway_downlink;
use wardriver_fleet_realtime::welcome_bundle::{WelcomeAttemptResult, WelcomeStage};
use wardriver_fleet_realtime::ESPNOW_PEER_IF;
use wardriver_packlink::RosterEntryV1;
use wardriver_stats as stats;

use crate::policy_downlink::{self, GatewayFleetState};

/// Arm or re-arm adoption FSM for current roster; cancel removed entries.
pub fn sync_roster(current: &[RosterEntryV1], prev: &[RosterEntryV1], now_ms: u64) {
    let current_ids: std::collections::HashSet<u32> =
        current.iter().map(|e| e.sniffer_id).collect();
    for entry in prev {
        if !current_ids.contains(&entry.sniffer_id) {
            gateway_adoption::cancel(entry.sniffer_id);
        }
    }
    for entry in current {
        gateway_adoption::arm(entry.sniffer_id, entry.mac, entry.fleet_slot, now_ms);
        if !gateway_adoption::is_confirmed(entry.sniffer_id) {
            stats::set_adoption_pending(entry.sniffer_id, entry.fleet_slot);
        }
    }
}

/// Periodic retry: deferred welcomes (Hello/role/policy) then timed FSM retries.
pub fn tick(fleet: &GatewayFleetState) {
    let now_ms = stats::now_ms();
    for sniffer_id in gateway_adoption::tick_timeouts(now_ms) {
        let fleet_slot = fleet_slot_for(fleet, sniffer_id).unwrap_or(0);
        stats::set_adoption_pending(sniffer_id, fleet_slot);
    }

    let mut attempts_left = gateway_adoption::MAX_WELCOME_ATTEMPTS_PER_TICK;
    let deferred_jobs = stats::take_pending_deferred_welcomes(now_ms);
    for job in deferred_jobs {
        if attempts_left == 0 {
            stats::enqueue_deferred_welcome(job.sniffer_id, job.mac, job.fleet_slot, job.force);
            continue;
        }
        if try_release_before_welcome(job.sniffer_id, job.mac, now_ms) {
            attempts_left = attempts_left.saturating_sub(1);
            continue;
        }
        if gateway_adoption::defer_welcome_for_release(job.sniffer_id, now_ms) {
            continue;
        }
        if let Err(e) = gateway_downlink::ensure_sniffer_peer_for_unicast(
            job.mac,
            ESPNOW_PEER_IF,
            "deferred welcome espnow peer sniffer",
        ) {
            warn!("gateway: adopt deferred welcome peer: {e:#}");
            gateway_adoption::record_welcome_outcome(
                job.sniffer_id,
                WelcomeAttemptResult::failure(WelcomeStage::EnsurePeer),
                now_ms,
            );
            stats::set_adoption_pending(job.sniffer_id, job.fleet_slot);
            attempts_left = attempts_left.saturating_sub(1);
            continue;
        }
        let result = policy_downlink::hello_welcome(fleet, job.mac, job.fleet_slot, job.force);
        gateway_adoption::record_welcome_outcome(job.sniffer_id, result, now_ms);
        if result.ok {
            stats::mark_deferred_welcome_sent(job.sniffer_id, now_ms);
            stats::set_adoption_pending(job.sniffer_id, job.fleet_slot);
        } else {
            warn!(
                "gateway: adopt deferred welcome sniffer_id={} stage={:?}",
                job.sniffer_id, result.failed_stage
            );
            stats::set_adoption_pending(job.sniffer_id, job.fleet_slot);
        }
        attempts_left = attempts_left.saturating_sub(1);
    }

    for (sniffer_id, mac, fleet_slot) in
        gateway_adoption::due_attempts_capped(now_ms, attempts_left)
    {
        if try_release_before_welcome(sniffer_id, mac, now_ms) {
            continue;
        }
        if gateway_adoption::defer_welcome_for_release(sniffer_id, now_ms) {
            continue;
        }
        let result = policy_downlink::hello_welcome(fleet, mac, fleet_slot, false);
        gateway_adoption::record_welcome_outcome(sniffer_id, result, now_ms);
        if result.ok {
            stats::set_adoption_pending(sniffer_id, fleet_slot);
        } else {
            warn!(
                "gateway: adopt welcome sniffer_id={sniffer_id} stage={:?}",
                result.failed_stage
            );
            stats::set_adoption_pending(sniffer_id, fleet_slot);
        }
    }

    if let Ok(policy) = fleet.policy.lock() {
        for entry in &policy.roster {
            if gateway_adoption::is_confirmed(entry.sniffer_id) {
                stats::set_adoption_adopted(entry.sniffer_id, entry.fleet_slot);
            }
        }
    }
}

/// Send `ReleaseBinding` once before welcome when the FSM requests takeover prep
/// (first arm or roster identity change — not PackLink reconnect / timeout reopen).
fn try_release_before_welcome(sniffer_id: u32, mac: [u8; 6], now_ms: u64) -> bool {
    if !gateway_adoption::needs_release_send(sniffer_id) {
        return false;
    }
    let release_epoch = now_ms.min(u64::from(u32::MAX)) as u32;
    match policy_downlink::send_release_binding_unicast(mac, release_epoch, ESPNOW_PEER_IF) {
        Ok(()) => gateway_adoption::note_release_sent(sniffer_id, now_ms),
        Err(e) => warn!("gateway: adopt release_binding sniffer_id={sniffer_id}: {e:#}"),
    }
    true
}

fn fleet_slot_for(fleet: &GatewayFleetState, sniffer_id: u32) -> Option<u8> {
    fleet
        .policy
        .lock()
        .ok()?
        .roster
        .iter()
        .find(|e| e.sniffer_id == sniffer_id)
        .map(|e| e.fleet_slot)
}
