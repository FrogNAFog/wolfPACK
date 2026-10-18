//! SnifferHello / SnifferAck / PeerJoin ESP-NOW handlers (control NVS roster adapter).

use std::collections::HashMap;

use indexmap::IndexMap;
use log::warn;

use crate::channel_plan;
use crate::fleet_roster;
use esp_idf_sys::wifi_interface_t_WIFI_IF_AP as ESPNOW_PEER_IF;
use wardriver_fleet_realtime::ingest::IngestPeerPoolState;
use wardriver_fleet_realtime::sniffer_ingress::{
    self, AckHooks, HelloHooks, RosterBinding, SnifferIngressCtx,
};
use wardriver_proto as proto;
use wardriver_stats as stats;

use super::espnow_rx::{self, DedupKey, Reasm, ReasmBle, ReasmKey, ReasmProbe};
use super::exclusion::drop_state_for_sniffer;
use super::fleet_cmd;
use super::ingress::sync_adoption_from_roster;
use super::ingress_ctx::ControlIngressCtx;

struct ControlRoster;

impl RosterBinding for ControlRoster {
    fn roster_allows(&self, sniffer_id: u32, mac: &[u8; 6]) -> bool {
        fleet_roster::with_roster(|r| r.is_adopted(sniffer_id, mac)).unwrap_or(false)
    }

    fn fleet_slot(&self, sniffer_id: u32) -> Option<u8> {
        fleet_roster::with_roster(|r| r.fleet_slot_for(sniffer_id)).flatten()
    }
}

struct ControlHelloHooks;

impl HelloHooks for ControlHelloHooks {
    fn adopted_for_stats(&self, sniffer_id: u32, peer: &[u8; 6]) -> bool {
        fleet_roster::with_roster(|r| r.is_adopted(sniffer_id, peer)).unwrap_or(false)
    }

    fn on_hello_not_rostered(&self, _sniffer_id: u32) {}

    fn on_hello_accepted(
        &self,
        sniffer_id: u32,
        mac: [u8; 6],
        fleet_slot: u8,
        rebooted: bool,
        _now_ms: u64,
    ) {
        stats::set_adoption_adopted(sniffer_id, fleet_slot);
        stats::enqueue_deferred_welcome(sniffer_id, mac, fleet_slot, rebooted);
    }
}

struct ControlIngressIngest<'a> {
    dedup: &'a mut IndexMap<DedupKey, u64>,
    dedup_ble: &'a mut IndexMap<DedupKey, u64>,
    dedup_probe: &'a mut IndexMap<DedupKey, u64>,
    reasm: &'a mut HashMap<ReasmKey, Reasm>,
    reasm_ble: &'a mut HashMap<ReasmKey, ReasmBle>,
    reasm_probe: &'a mut HashMap<ReasmKey, ReasmProbe>,
}

impl IngestPeerPoolState for ControlIngressIngest<'_> {
    fn has_incomplete_reasm_for_mac(&self, mac: &[u8; 6]) -> bool {
        espnow_rx::has_incomplete_reasm_for_mac(mac, self.reasm, self.reasm_ble)
    }

    fn drop_state_for_sniffer(&mut self, sniffer_id: u32) {
        drop_state_for_sniffer(
            sniffer_id,
            self.dedup,
            self.dedup_ble,
            self.dedup_probe,
            self.reasm,
            self.reasm_ble,
            self.reasm_probe,
        );
    }
}

struct ControlAckHooks<'a> {
    control_boot_id: u32,
    sd_io: &'a crate::storage::SdIoHandle,
}

impl AckHooks for ControlAckHooks<'_> {
    fn on_ack_after_seen(&self, sniffer_id: u32, peer: &[u8; 6], _ack: &proto::SnifferAckV1) {
        sync_adoption_from_roster(sniffer_id, peer);
    }

    fn on_role_reconcile_clear_ble(&self, sniffer_id: u32) {
        stats::clear_sniffer_plan_assignment(sniffer_id);
        channel_plan::remove_sniffer_from_plan_lists(sniffer_id, self.sd_io);
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
        if let Err(e) = fleet_cmd::send_setrole_unicast(
            peer,
            desired_role,
            cmd_seq,
            self.control_boot_id,
            ESPNOW_PEER_IF,
            Some(stats::time_set_seq_for_wire()),
        ) {
            warn!("control: ack SetRole resend: {e:#}");
        } else if let Some(sid) = stats::sniffer_id_for_mac(&peer) {
            stats::mark_setrole_sent(sid, stats::now_ms());
        }
    }
}

pub(super) fn uplink_liveness_and_boot_ok(
    ctx: &mut ControlIngressCtx<'_>,
    sniffer_id: u32,
    peer: &[u8; 6],
    sniffer_boot_id: u32,
) -> bool {
    sync_adoption_from_roster(sniffer_id, peer);
    let ControlIngressCtx {
        cmd_seq,
        control_boot_id,
        last_sniffer_boot_id,
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
        ..
    } = ctx;
    let mut ingest = ControlIngressIngest {
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
    };
    let mut sctx = SnifferIngressCtx {
        control_boot_id: *control_boot_id,
        cmd_seq,
        last_sniffer_boot_id,
        ingest: &mut ingest,
    };
    sniffer_ingress::uplink_liveness_and_boot_ok(
        &mut sctx,
        sniffer_id,
        peer,
        sniffer_boot_id,
        "control",
    )
}

pub(super) fn handle_sniffer_hello(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    let ControlIngressCtx {
        cmd_seq,
        control_boot_id,
        last_sniffer_boot_id,
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
        ..
    } = ctx;
    let mut ingest = ControlIngressIngest {
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
    };
    let mut sctx = SnifferIngressCtx {
        control_boot_id: *control_boot_id,
        cmd_seq,
        last_sniffer_boot_id,
        ingest: &mut ingest,
    };
    if let Some(peer_mac) = sniffer_ingress::handle_sniffer_hello(
        &mut sctx,
        &ControlRoster,
        &ControlHelloHooks,
        peer,
        data,
        "control",
    ) {
        use wardriver_fleet_realtime::gateway_downlink;
        if let Err(e) = gateway_downlink::ensure_sniffer_peer_for_unicast(
            peer_mac,
            ESPNOW_PEER_IF,
            "hello path espnow peer sniffer",
        ) {
            warn!("control: {e:#}");
        }
    }
}

pub(super) fn handle_peer_join(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    let ControlIngressCtx {
        cmd_seq,
        control_boot_id,
        last_sniffer_boot_id,
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
        ..
    } = ctx;
    let mut ingest = ControlIngressIngest {
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
    };
    let mut sctx = SnifferIngressCtx {
        control_boot_id: *control_boot_id,
        cmd_seq,
        last_sniffer_boot_id,
        ingest: &mut ingest,
    };
    sniffer_ingress::handle_peer_join(&mut sctx, &ControlRoster, peer, data, "control");
}

pub(super) fn handle_sniffer_ack(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    let ControlIngressCtx {
        cmd_seq,
        control_boot_id,
        last_sniffer_boot_id,
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
        sd_io,
        ..
    } = ctx;
    let mut ingest = ControlIngressIngest {
        dedup,
        dedup_ble,
        dedup_probe,
        reasm,
        reasm_ble,
        reasm_probe,
    };
    let mut sctx = SnifferIngressCtx {
        control_boot_id: *control_boot_id,
        cmd_seq,
        last_sniffer_boot_id,
        ingest: &mut ingest,
    };
    let hooks = ControlAckHooks {
        control_boot_id: *control_boot_id,
        sd_io,
    };
    sniffer_ingress::handle_sniffer_ack(&mut sctx, &ControlRoster, &hooks, peer, data, "control");
}
