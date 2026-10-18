//! SnifferHello / SnifferAck / PeerJoin ESP-NOW handlers (phone-owned roster).
//!
//! Gateway adapter over shared [`sniffer_ingress`].

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;

use log::warn;

use wardriver_packlink::PolicySyncV1;
use wardriver_proto as proto;
use wardriver_stats as stats;

use crate::gateway_adoption;
use wardriver_fleet_realtime::gateway_downlink;
use wardriver_fleet_realtime::ingest::IngestPeerPoolState;
use wardriver_fleet_realtime::roster_gate::{roster_allows, roster_fleet_slot};
use wardriver_fleet_realtime::sniffer_ingress::{
    self, AckHooks, HelloHooks, RosterBinding, SnifferIngressCtx,
};
use wardriver_fleet_realtime::ESPNOW_PEER_IF;

/// Mutable per-visit state for gateway ESP-NOW ingest.
pub struct GatewayIngressCtx<'a, I: IngestPeerPoolState + ?Sized> {
    pub policy: &'a PolicySyncV1,
    pub control_boot_id: u32,
    pub cmd_seq: &'a AtomicU32,
    pub last_sniffer_boot_id: &'a mut HashMap<u32, u32>,
    pub ingest: &'a mut I,
}

impl<'a, I: IngestPeerPoolState + ?Sized> GatewayIngressCtx<'a, I> {
    fn sniffer_ctx(&mut self) -> SnifferIngressCtx<'_, I> {
        SnifferIngressCtx {
            control_boot_id: self.control_boot_id,
            cmd_seq: self.cmd_seq,
            last_sniffer_boot_id: self.last_sniffer_boot_id,
            ingest: self.ingest,
        }
    }
}

struct PolicyRoster<'a> {
    policy: &'a PolicySyncV1,
}

impl RosterBinding for PolicyRoster<'_> {
    fn roster_allows(&self, sniffer_id: u32, mac: &[u8; 6]) -> bool {
        roster_allows(self.policy, sniffer_id, mac)
    }

    fn fleet_slot(&self, sniffer_id: u32) -> Option<u8> {
        roster_fleet_slot(self.policy, sniffer_id)
    }
}

struct GatewayHelloHooks;

impl HelloHooks for GatewayHelloHooks {
    fn adopted_for_stats(&self, sniffer_id: u32, _peer: &[u8; 6]) -> bool {
        gateway_adoption::is_confirmed(sniffer_id)
    }

    fn on_hello_not_rostered(&self, sniffer_id: u32) {
        gateway_adoption::note_hello(sniffer_id, stats::now_ms());
    }

    fn on_hello_accepted(
        &self,
        sniffer_id: u32,
        mac: [u8; 6],
        fleet_slot: u8,
        rebooted: bool,
        now_ms: u64,
    ) {
        gateway_adoption::note_hello(sniffer_id, now_ms);
        let confirmed = gateway_adoption::is_confirmed(sniffer_id);
        if confirmed {
            stats::set_adoption_adopted(sniffer_id, fleet_slot);
        } else {
            stats::set_adoption_pending(sniffer_id, fleet_slot);
        }
        // Confirmed + same sniffer_boot_id: liveness only — no welcome/peer churn.
        if rebooted || !confirmed {
            stats::enqueue_deferred_welcome(sniffer_id, mac, fleet_slot, rebooted);
            gateway_adoption::request_welcome_now(sniffer_id, now_ms);
        }
    }
}

struct GatewayAckHooks {
    policy: *const PolicySyncV1,
    control_boot_id: u32,
}

impl GatewayAckHooks {
    fn policy(&self) -> &PolicySyncV1 {
        // SAFETY: policy outlives handler call from GatewayIngressCtx.
        unsafe { &*self.policy }
    }
}

impl AckHooks for GatewayAckHooks {
    fn on_ack_after_seen(&self, sniffer_id: u32, peer: &[u8; 6], ack: &proto::SnifferAckV1) {
        gateway_adoption::note_orphan_ack(sniffer_id);
        let confirmed_now = gateway_adoption::try_confirm(sniffer_id, peer, ack.ack_cmd_seq);
        if confirmed_now || gateway_adoption::is_confirmed(sniffer_id) {
            if let Some(slot) = roster_fleet_slot(self.policy(), sniffer_id) {
                stats::set_adoption_adopted(sniffer_id, slot);
            }
        }
    }

    fn on_role_reconcile_clear_ble(&self, sniffer_id: u32) {
        stats::clear_sniffer_plan_assignment(sniffer_id);
    }

    fn reconcile_role_after_ack(
        &self,
        sniffer_id: u32,
        role: u8,
        now_ms: u64,
    ) -> stats::PendingRoleReconcile {
        stats::reconcile_pending_role_after_ack(sniffer_id, role, now_ms)
    }

    fn resend_setrole(&self, peer: [u8; 6], desired_role: u8, cmd_seq: u32) {
        if let Err(e) = gateway_downlink::send_setrole_unicast(
            peer,
            desired_role,
            cmd_seq,
            self.control_boot_id,
            ESPNOW_PEER_IF,
            Some(stats::time_set_seq_for_wire()),
        ) {
            warn!("gateway: ack SetRole resend: {e:#}");
        } else if let Some(sid) = stats::sniffer_id_for_mac(&peer) {
            stats::mark_setrole_sent(sid, stats::now_ms());
        }
    }
}

pub fn uplink_liveness_and_boot_ok<I: IngestPeerPoolState + ?Sized>(
    ctx: &mut GatewayIngressCtx<'_, I>,
    sniffer_id: u32,
    peer: &[u8; 6],
    sniffer_boot_id: u32,
) -> bool {
    sniffer_ingress::uplink_liveness_and_boot_ok(
        &mut ctx.sniffer_ctx(),
        sniffer_id,
        peer,
        sniffer_boot_id,
        "gateway",
    )
}

pub fn handle_sniffer_hello<I: IngestPeerPoolState + ?Sized>(
    ctx: &mut GatewayIngressCtx<'_, I>,
    peer: [u8; 6],
    data: &[u8],
) -> Option<[u8; 6]> {
    let roster = PolicyRoster { policy: ctx.policy };
    sniffer_ingress::handle_sniffer_hello(
        &mut ctx.sniffer_ctx(),
        &roster,
        &GatewayHelloHooks,
        peer,
        data,
        "gateway",
    )
}

pub fn handle_peer_join<I: IngestPeerPoolState + ?Sized>(
    ctx: &mut GatewayIngressCtx<'_, I>,
    peer: [u8; 6],
    data: &[u8],
) {
    let roster = PolicyRoster { policy: ctx.policy };
    sniffer_ingress::handle_peer_join(&mut ctx.sniffer_ctx(), &roster, peer, data, "gateway");
}

pub fn handle_sniffer_ack<I: IngestPeerPoolState + ?Sized>(
    ctx: &mut GatewayIngressCtx<'_, I>,
    peer: [u8; 6],
    data: &[u8],
) {
    let roster = PolicyRoster { policy: ctx.policy };
    let hooks = GatewayAckHooks {
        policy: std::ptr::from_ref(ctx.policy),
        control_boot_id: ctx.control_boot_id,
    };
    sniffer_ingress::handle_sniffer_ack(
        &mut ctx.sniffer_ctx(),
        &roster,
        &hooks,
        peer,
        data,
        "gateway",
    );
}
