//! ESP-NOW wolfPACK downlink: peer ensure, control commands, post-dump done.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use esp_idf_sys::wifi_interface_t;
use log::warn;
use std::time::Duration;

use wardriver_proto::{self as proto, ControlCmd, ControlCmdV1};
use wardriver_shared::espnow;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::fleet_commands::FleetList;
use crate::peer_pool;
use crate::CONTROL_AP_CHANNEL;

/// Wall-clock hooks for piggybacked `time_set_seq` on broadcast Ping.
pub struct GpsTimeHooks {
    pub wall_trusted: fn() -> bool,
    pub unix_now_ms: fn() -> Option<u64>,
}

static GPS_TIME: OnceLock<GpsTimeHooks> = OnceLock::new();

/// Register GPS / wall-clock providers (control: GPS+SNTP; gateway: phone TimeSync).
pub fn register_gps_time_hooks(hooks: GpsTimeHooks) {
    let _ = GPS_TIME.set(hooks);
}

/// Minimum interval between bumping piggybacked `time_set_seq` on broadcast Ping.
pub const TIME_SET_SEQ_INTERVAL_MS: u64 = 2_000;

/// Register sniffer MAC before encrypted unicast (wolfPACK peer pool + LRU cap).
pub fn ensure_sniffer_peer_for_unicast(
    mac: [u8; 6],
    peer_if: wifi_interface_t,
    context: &str,
) -> Result<()> {
    peer_pool::ensure_peer(mac, CONTROL_AP_CHANNEL, peer_if)
        .map_err(|e| anyhow!("{context}: {e:#}"))
}

/// Attach GPS wall time when trusted; `seq == 0` leaves fields at zero (no piggyback).
pub fn attach_gps_time(cmd: &mut ControlCmdV1, seq: u32) {
    if seq == 0 {
        return;
    }
    let Some(hooks) = GPS_TIME.get() else {
        return;
    };
    if !(hooks.wall_trusted)() {
        return;
    }
    let Some(unix_ms) = (hooks.unix_now_ms)() else {
        return;
    };
    if unix_ms == 0 {
        return;
    }
    cmd.time_set_seq = seq;
    cmd.ctrl_unix_ms = unix_ms;
}

/// Bump piggybacked `time_set_seq` when the ping interval elapsed; returns seq to attach (if any).
pub fn bump_time_set_seq_if_due(last_bump_ms: &mut u64, now_ms: u64) -> Option<u32> {
    if now_ms.saturating_sub(*last_bump_ms) < TIME_SET_SEQ_INTERVAL_MS {
        return None;
    }
    *last_bump_ms = now_ms;
    let seq = stats::TIME_SET_SEQ.fetch_add(1, Ordering::Relaxed);
    Some(seq)
}

/// Dwell and schedule fields for a fleet control command.
#[derive(Clone, Copy, Debug)]
pub struct FleetCommandBuilder {
    pub dwell_ms: u16,
}

impl FleetCommandBuilder {
    #[must_use]
    pub const fn new(dwell_ms: u16) -> Self {
        Self { dwell_ms }
    }

    pub fn control_cmd_v1(
        &self,
        cmd_seq: u32,
        cmd: ControlCmd,
        schedule: Option<([u8; proto::CHANNELS_MAX], u8)>,
        control_boot_id: u32,
        gps_time_seq: Option<u32>,
        desired_scanning: u8,
    ) -> ControlCmdV1 {
        let (channels, chan_count) = schedule.unwrap_or(([0u8; proto::CHANNELS_MAX], 0));
        let mut out = ControlCmdV1 {
            cmd,
            cmd_seq,
            control_channel: CONTROL_AP_CHANNEL,
            dwell_ms: self.dwell_ms,
            chan_count,
            channels,
            desired_role: 0,
            control_boot_id,
            detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
            wardrive_suppressed: u8::from(stats::wardrive_suppressed()),
            time_set_seq: 0,
            ctrl_unix_ms: 0,
            detection_disable_ble_mask: stats::detection_disable_ble_mask(),
            desired_scanning: desired_scanning.min(1),
            peer_slots_free: 0,
            peer_join_waiters: 0,
            remote_id_disable_mask: stats::remote_id_disable_mask(),
            ble_active_scan: u8::from(stats::ble_active_scan()),
        };
        if let Some(seq) = gps_time_seq {
            attach_gps_time(&mut out, seq);
        }
        out
    }
}

/// Build a broadcast `Ping` command (PROTO v7 short wire; no channel plan).
pub fn build_ping_cmd_v1(
    cmd_seq: u32,
    control_boot_id: u32,
    gps_time_seq: Option<u32>,
    desired_scanning: u8,
) -> ControlCmdV1 {
    let mut out = ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq,
        control_channel: CONTROL_AP_CHANNEL,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; proto::CHANNELS_MAX],
        desired_role: 0,
        control_boot_id,
        detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
        wardrive_suppressed: u8::from(stats::wardrive_suppressed()),
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: stats::detection_disable_ble_mask(),
        desired_scanning: desired_scanning.min(1),
        peer_slots_free: peer_pool::peer_slots_free_estimate(),
        peer_join_waiters: peer_pool::join_queue_waiter_count(),
        remote_id_disable_mask: stats::remote_id_disable_mask(),
        ble_active_scan: u8::from(stats::ble_active_scan()),
    };
    if let Some(seq) = gps_time_seq {
        attach_gps_time(&mut out, seq);
    }
    out
}

fn pack_control_payload(cmd: ControlCmdV1) -> Result<Vec<u8>> {
    if cmd.cmd == ControlCmd::Ping {
        proto::pack_ping_vec(cmd)
    } else {
        proto::pack_control_cmd_vec(cmd)
    }
}

pub fn send_setrole_unicast(
    mac: [u8; 6],
    desired_role: u8,
    cmd_seq: u32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    gps_time_seq: Option<u32>,
) -> Result<()> {
    let mut cmd = ControlCmdV1 {
        cmd: ControlCmd::SetRole,
        cmd_seq,
        control_channel: CONTROL_AP_CHANNEL,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; proto::CHANNELS_MAX],
        desired_role,
        control_boot_id,
        detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
        wardrive_suppressed: u8::from(stats::wardrive_suppressed()),
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: stats::detection_disable_ble_mask(),
        desired_scanning: 0,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: stats::remote_id_disable_mask(),
        ble_active_scan: u8::from(stats::ble_active_scan()),
    };
    if let Some(seq) = gps_time_seq {
        attach_gps_time(&mut cmd, seq);
    }
    send_cmd_unicast(mac, cmd, peer_if)
}

/// Unicast `PostDumpDone` after post-dump flush + ack wait.
pub fn send_post_dump_done(
    mac: [u8; 6],
    sniffer_id: u32,
    session_seq: u32,
    control_tx_ms: u32,
    peer_if: wifi_interface_t,
) -> Result<()> {
    let wire = proto::pack_post_dump_done(proto::PostDumpDoneV1 {
        sniffer_id,
        session_seq,
        control_tx_ms,
    });
    ensure_sniffer_peer_for_unicast(mac, peer_if, "post_dump_done ensure_peer")?;
    espnow::send(mac, &wire).map_err(|e| anyhow!("post_dump_done send: {e:#}"))
}

pub fn send_cmd_unicast(mac: [u8; 6], cmd: ControlCmdV1, peer_if: wifi_interface_t) -> Result<()> {
    let payload = pack_control_payload(cmd)?;
    ensure_sniffer_peer_for_unicast(mac, peer_if, "espnow peer sniffer (unicast cmd)")?;
    espnow::send(mac, &payload).map_err(|e| anyhow!("espnow send unicast cmd: {e:#}"))
}

/// Broadcast a packed denylist/watchlist while preserving broadcast-only peer usage.
pub fn broadcast_fleet_list(
    list: FleetList<'_>,
    peer_if: wifi_interface_t,
    context: &str,
) -> Result<()> {
    let payload = list.pack()?;
    if let Err(e) = espnow::add_broadcast_peer(CONTROL_AP_CHANNEL, peer_if) {
        warn!("fleet-realtime: {context} add_broadcast_peer: {e:#}");
    }
    if let Err(e) = espnow::send(espnow::BROADCAST_MAC, &payload) {
        warn!("fleet-realtime: {context} send broadcast: {e:#}");
    }
    Ok(())
}

/// Send a packed denylist/watchlist to one encrypted fleet peer.
pub fn send_fleet_list_unicast(
    mac: [u8; 6],
    list: FleetList<'_>,
    peer_if: wifi_interface_t,
    context: &str,
) -> Result<()> {
    let payload = list.pack()?;
    ensure_sniffer_peer_for_unicast(mac, peer_if, context)?;
    espnow::send(mac, &payload).map_err(|e| anyhow!("{context}: {e:#}"))
}

/// Drain first-send SetRole jobs with shared sequencing and ACK bookkeeping.
pub fn drain_pending_role_changes(
    cmd_seq: &AtomicU32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    gps_time_seq: Option<u32>,
    mut on_error: impl FnMut(u32, u8, &anyhow::Error),
) {
    let now_ms = stats::now_ms();
    for (sniffer_id, mac, desired_role) in stats::pending_roles_needing_first_send() {
        let seq = cmd_seq.fetch_add(1, Ordering::Relaxed);
        match send_setrole_unicast(
            mac,
            desired_role,
            seq,
            control_boot_id,
            peer_if,
            gps_time_seq,
        ) {
            Ok(()) => stats::mark_setrole_sent(sniffer_id, now_ms),
            Err(e) => on_error(sniffer_id, desired_role, &e),
        }
    }
}

/// Drain deferred welcome jobs in queue order; the role callback controls success marking.
pub fn drain_deferred_welcomes(
    peer_if: wifi_interface_t,
    context: &str,
    mut send: impl FnMut(&stats::PendingDeferredWelcome) -> bool,
) {
    let now_ms = stats::now_ms();
    for job in stats::take_pending_deferred_welcomes(now_ms) {
        if let Err(e) = ensure_sniffer_peer_for_unicast(job.mac, peer_if, context) {
            warn!("fleet-realtime: {e:#}");
            continue;
        }
        if send(&job) {
            stats::mark_deferred_welcome_sent(job.sniffer_id, now_ms);
        }
    }
}

/// Send a control command (broadcast and optional per-sniffer unicast).
pub fn broadcast_cmd(
    sniffers: &Arc<Mutex<HashSet<[u8; 6]>>>,
    seq: &AtomicU32,
    cmd: ControlCmd,
    schedule: Option<([u8; proto::CHANNELS_MAX], u8)>,
    peer_if: wifi_interface_t,
    control_boot_id: u32,
    broadcast_only: bool,
    builder: &FleetCommandBuilder,
    gps_time_seq: Option<u32>,
    desired_scanning: u8,
) -> Result<()> {
    let cmd_seq = seq.fetch_add(1, Ordering::Relaxed);
    if matches!(cmd, ControlCmd::Ping) {
        stats::set_last_control_ping_cmd_seq(cmd_seq);
    }
    let cmd_v1 = if matches!(cmd, ControlCmd::Ping) {
        build_ping_cmd_v1(cmd_seq, control_boot_id, gps_time_seq, desired_scanning)
    } else {
        builder.control_cmd_v1(
            cmd_seq,
            cmd,
            schedule,
            control_boot_id,
            gps_time_seq,
            desired_scanning,
        )
    };
    let payload = pack_control_payload(cmd_v1)?;
    if !broadcast_only {
        let peers: Vec<[u8; 6]> = sniffers
            .lock()
            .ok()
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for mac in peers {
            if let Err(e) = espnow::add_peer(mac, CONTROL_AP_CHANNEL, peer_if) {
                warn!("fleet-realtime: espnow add_peer sniffer: {e:#}");
            }
            if let Err(e) = espnow::send(mac, &payload) {
                warn!("fleet-realtime: espnow send unicast: {e:#}");
            }
        }
    }

    let track_ping_bcast_fail = broadcast_only && matches!(cmd, ControlCmd::Ping);
    if let Err(e) = espnow::add_broadcast_peer(CONTROL_AP_CHANNEL, peer_if) {
        warn!("fleet-realtime: espnow add_broadcast_peer: {e:#}");
        if track_ping_bcast_fail {
            stats::ESPNOW_PERIODIC_PING_BROADCAST_FAIL.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Err(e) = espnow::send(espnow::BROADCAST_MAC, &payload) {
        warn!("fleet-realtime: espnow send broadcast: {e:#}");
        if track_ping_bcast_fail {
            stats::ESPNOW_PERIODIC_PING_BROADCAST_FAIL.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Retries per MAC when fanning out `ReleaseBinding`.
pub const RELEASE_BINDING_RETRIES: u8 = 3;

/// Delay between `ReleaseBinding` retries (ms).
const RELEASE_BINDING_RETRY_GAP_MS: u64 = 150;

/// ESP-NOW unicast `ReleaseBinding` to one sniffer MAC.
pub fn send_release_binding_unicast(
    mac: [u8; 6],
    release_epoch: u32,
    peer_if: wifi_interface_t,
) -> Result<()> {
    let payload = proto::pack_release_binding(release_epoch);
    ensure_sniffer_peer_for_unicast(mac, peer_if, "espnow peer sniffer (release binding)")?;
    espnow::send(mac, &payload).map_err(|e| anyhow!("espnow send release binding: {e:#}"))
}

/// Unicast `ReleaseBinding` to each MAC; returns count of MACs that acked send OK on any attempt.
pub fn release_bindings_for_macs(
    macs: &[[u8; 6]],
    peer_if: wifi_interface_t,
    retries: u8,
) -> usize {
    let release_epoch = stats::now_ms() as u32;
    let attempts = retries.max(1);
    let mut ok = 0usize;
    for mac in macs {
        let mut sent = false;
        for attempt in 0..attempts {
            match send_release_binding_unicast(*mac, release_epoch, peer_if) {
                Ok(()) => {
                    sent = true;
                    break;
                }
                Err(e) => {
                    warn!(
                        "fleet-realtime: release_binding {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} attempt {}: {e:#}",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
                        attempt + 1
                    );
                    if attempt + 1 < attempts {
                        task_wdt::sleep_feeding_wdt(Duration::from_millis(
                            RELEASE_BINDING_RETRY_GAP_MS,
                        ));
                    }
                }
            }
        }
        if sent {
            ok += 1;
        }
    }
    ok
}
