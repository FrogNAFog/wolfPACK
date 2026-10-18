//! Shared sniffer Hello / PeerJoin / Ack ingress (control + gateway adapters).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::ingest::IngestPeerPoolState;
use crate::sniffer_lifecycle::hello_gate;
use crate::CONTROL_AP_CHANNEL;
use crate::ESPNOW_PEER_IF;
use log::{info, warn};

use wardriver_proto as proto;
use wardriver_shared::espnow;
use wardriver_stats as stats;

use crate::peer_pool;

/// Roster admission: exact (sniffer_id, MAC) match and fleet slot lookup.
pub trait RosterBinding {
    fn roster_allows(&self, sniffer_id: u32, mac: &[u8; 6]) -> bool;
    fn fleet_slot(&self, sniffer_id: u32) -> Option<u8>;
}

/// Role-specific Hello side effects (deferred welcome, adoption FSM, stats).
pub trait HelloHooks {
    /// Dashboard "adopted" bit for `sniffer_seen_minimal`.
    fn adopted_for_stats(&self, sniffer_id: u32, peer: &[u8; 6]) -> bool;
    fn on_hello_not_rostered(&self, sniffer_id: u32);
    fn on_hello_accepted(
        &self,
        sniffer_id: u32,
        mac: [u8; 6],
        fleet_slot: u8,
        rebooted: bool,
        now_ms: u64,
    );
}

/// Role-specific Ack side effects (adoption confirm, BLE plan cleanup, SetRole resend).
pub trait AckHooks {
    fn on_ack_after_seen(&self, sniffer_id: u32, mac: &[u8; 6], ack: &proto::SnifferAckV1);
    fn on_role_reconcile_clear_ble(&self, sniffer_id: u32);
    fn reconcile_role_after_ack(
        &self,
        sniffer_id: u32,
        role: u8,
        now_ms: u64,
    ) -> stats::PendingRoleReconcile;
    fn resend_setrole(&self, peer: [u8; 6], desired_role: u8, cmd_seq: u32);
}

/// Mutable per-visit ingress state shared by control and gateway.
pub struct SnifferIngressCtx<'a, I: IngestPeerPoolState + ?Sized> {
    pub control_boot_id: u32,
    pub cmd_seq: &'a AtomicU32,
    pub last_sniffer_boot_id: &'a mut HashMap<u32, u32>,
    pub ingest: &'a mut I,
}

fn note_boot_and_maybe_drop<I: IngestPeerPoolState + ?Sized>(
    ctx: &mut SnifferIngressCtx<'_, I>,
    sniffer_id: u32,
    boot_id: u32,
    role: &'static str,
) -> bool {
    let map_reboot = match ctx.last_sniffer_boot_id.insert(sniffer_id, boot_id) {
        Some(prev) if prev != boot_id => {
            info!(
                "{role}: sniffer_id={sniffer_id} sniffer_boot_id {prev:#x} -> {boot_id:#x}; sniffer rebooted"
            );
            true
        }
        _ => false,
    };
    let slot_reboot = stats::note_sniffer_boot_id(sniffer_id, boot_id);
    if map_reboot || slot_reboot {
        ctx.ingest.drop_state_for_sniffer(sniffer_id);
        return true;
    }
    false
}

/// Refresh dashboard liveness; return false when payload should be dropped (boot mismatch).
pub fn uplink_liveness_and_boot_ok<I: IngestPeerPoolState + ?Sized>(
    ctx: &mut SnifferIngressCtx<'_, I>,
    sniffer_id: u32,
    peer: &[u8; 6],
    sniffer_boot_id: u32,
    role: &'static str,
) -> bool {
    stats::sniffer_seen_liveness(sniffer_id, peer);
    match ctx.last_sniffer_boot_id.get(&sniffer_id).copied() {
        Some(boot) if boot != sniffer_boot_id => {
            note_boot_and_maybe_drop(ctx, sniffer_id, sniffer_boot_id, role);
            false
        }
        None => {
            note_boot_and_maybe_drop(ctx, sniffer_id, sniffer_boot_id, role);
            true
        }
        Some(_) => true,
    }
}

pub fn handle_sniffer_hello<I: IngestPeerPoolState + ?Sized, R: RosterBinding, H: HelloHooks>(
    ctx: &mut SnifferIngressCtx<'_, I>,
    roster: &R,
    hooks: &H,
    peer: [u8; 6],
    data: &[u8],
    role: &'static str,
) -> Option<[u8; 6]> {
    let Ok(h) = proto::unpack_sniffer_hello(data) else {
        stats::INGRESS_HELLO_UNPACK_FAIL.fetch_add(1, Ordering::Relaxed);
        warn!(
            "{role}: unpack_sniffer_hello failed len={} peer={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            data.len(),
            peer[0], peer[1], peer[2], peer[3], peer[4], peer[5],
        );
        return None;
    };
    let rebooted = note_boot_and_maybe_drop(ctx, h.sniffer_id, h.sniffer_boot_id, role);

    let adopted = hooks.adopted_for_stats(h.sniffer_id, &peer);
    stats::sniffer_seen_minimal(h.sniffer_id, &peer, adopted);

    let Some(gate) = hello_gate(
        roster.roster_allows(h.sniffer_id, &peer),
        roster.fleet_slot(h.sniffer_id),
        rebooted,
    ) else {
        warn!(
            "{role}: hello rostered sniffer_id={} missing fleet_slot",
            h.sniffer_id
        );
        return None;
    };
    match gate {
        crate::sniffer_lifecycle::HelloGate::DropNotRostered => {
            stats::INGRESS_HELLO_ADOPT_PENDING.fetch_add(1, Ordering::Relaxed);
            hooks.on_hello_not_rostered(h.sniffer_id);
            None
        }
        crate::sniffer_lifecycle::HelloGate::Accept {
            fleet_slot,
            rebooted,
        } => {
            let now_ms = stats::now_ms();
            hooks.on_hello_accepted(h.sniffer_id, peer, fleet_slot, rebooted, now_ms);
            Some(peer)
        }
    }
}

pub fn handle_peer_join<I: IngestPeerPoolState + ?Sized, R: RosterBinding>(
    ctx: &mut SnifferIngressCtx<'_, I>,
    roster: &R,
    peer: [u8; 6],
    data: &[u8],
    role: &'static str,
) {
    let Ok(join) = proto::unpack_peer_join(data) else {
        return;
    };
    note_boot_and_maybe_drop(ctx, join.sniffer_id, join.sniffer_boot_id, role);

    if !roster.roster_allows(join.sniffer_id, &peer) {
        stats::INGRESS_ROSTER_DROP.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Some(fleet_slot) = roster.fleet_slot(join.sniffer_id) else {
        warn!(
            "{role}: peer_join rostered sniffer_id={} missing fleet_slot",
            join.sniffer_id
        );
        return;
    };
    stats::sniffer_seen_minimal(join.sniffer_id, &peer, true);
    peer_pool::note_peer_join_attempt(
        peer,
        join.sniffer_id,
        stats::now_ms(),
        join.join_reason as u8,
    );

    if let Err(nack) =
        peer_pool::try_admit_peer_join(join.sniffer_id, peer, CONTROL_AP_CHANNEL, ESPNOW_PEER_IF)
    {
        peer_pool::send_peer_join_nack(join.sniffer_id, &nack, ESPNOW_PEER_IF);
        return;
    }

    let wire = proto::pack_peer_join_ack(proto::PeerJoinAckV1 {
        sniffer_id: join.sniffer_id,
        fleet_slot,
        peer_epoch: ctx.control_boot_id,
    });
    if let Err(e) = espnow::send(peer, &wire) {
        warn!(
            "{role}: peer_join_ack send sniffer_id={}: {e:#}",
            join.sniffer_id
        );
        let now_ms = stats::now_ms();
        let nack = peer_pool::abort_peer_join_after_failed_ack(join.sniffer_id, peer, now_ms);
        peer_pool::send_peer_join_nack(join.sniffer_id, &nack, ESPNOW_PEER_IF);
        return;
    }
    let now_ms = stats::now_ms();
    peer_pool::confirm_peer_join_admitted(join.sniffer_id, peer, now_ms);
    peer_pool::begin_post_dump_visit(peer, join.sniffer_id, now_ms, join.join_reason as u8);
}

pub fn handle_sniffer_ack<I: IngestPeerPoolState + ?Sized, R: RosterBinding, A: AckHooks>(
    ctx: &mut SnifferIngressCtx<'_, I>,
    roster: &R,
    hooks: &A,
    peer: [u8; 6],
    data: &[u8],
    role: &'static str,
) {
    let Ok(ack) = proto::unpack_sniffer_ack(data) else {
        return;
    };
    note_boot_and_maybe_drop(ctx, ack.sniffer_id, ack.sniffer_boot_id, role);
    stats::sniffer_seen_from_ack(ack.sniffer_id, &peer, &ack);
    if !roster.roster_allows(ack.sniffer_id, &peer) {
        return;
    }
    hooks.on_ack_after_seen(ack.sniffer_id, &peer, &ack);
    let now_ms = stats::now_ms();
    peer_pool::complete_peer_refresh_visit(peer, now_ms);
    match hooks.reconcile_role_after_ack(ack.sniffer_id, ack.role as u8, now_ms) {
        stats::PendingRoleReconcile::Clear => {
            if ack.role as u8 == 1 {
                hooks.on_role_reconcile_clear_ble(ack.sniffer_id);
            }
        }
        stats::PendingRoleReconcile::Resend { desired_role } => {
            let cmd_seq_now = ctx.cmd_seq.fetch_add(1, Ordering::Relaxed);
            hooks.resend_setrole(peer, desired_role, cmd_seq_now);
        }
        _ => {}
    }
}
