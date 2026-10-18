//! ESP-NOW fan-out: BSSID denylist and control commands to sniffers.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{anyhow, Result};
use esp_idf_sys::wifi_interface_t;
use log::warn;

use crate::channel_plan;
use crate::config;
use crate::time_sync;
use wardriver_fleet_realtime::fleet_commands::{
    build_start_stop, build_welcome, FleetList, StartStopConfig, WelcomeConfig,
};
use wardriver_fleet_realtime::gateway_downlink::{self, GpsTimeHooks};
use wardriver_fleet_realtime::welcome_downlink::{
    run_welcome_bundle, WelcomeDownlinkCtx, WelcomeDownlinkHooks,
};
use wardriver_proto::{self as proto, ControlCmd, ControlCmdV1};
use wardriver_shared::espnow;
use wardriver_stats as stats;

use super::csv_ingest::unix_now_ms;
use super::fleet_state::fill_active_wifi_sniffers;
use super::CONTROL_AP_CHANNEL;

pub(crate) use gateway_downlink::{
    attach_gps_time, broadcast_cmd, ensure_sniffer_peer_for_unicast, release_bindings_for_macs,
    send_cmd_unicast, send_release_binding_unicast, send_setrole_unicast, FleetCommandBuilder,
    RELEASE_BINDING_RETRIES,
};

/// Register control GPS wall-clock hooks for downlink piggyback (call once at boot).
pub(crate) fn register_control_gps_hooks() {
    gateway_downlink::register_gps_time_hooks(GpsTimeHooks {
        wall_trusted: || time_sync::wall_clock_trusted(),
        unix_now_ms: || unix_now_ms(),
    });
}

/// Builder with dwell from the live channel plan.
#[must_use]
pub(crate) fn fleet_cmd_builder_from_plan() -> FleetCommandBuilder {
    FleetCommandBuilder::new(wardriver_core::dwell::DwellAndSchedule::dwell_ms_from_plan(
        &channel_plan::current_plan(),
    ))
}

/// Push merged WiGLE BSSID denylist to sniffers (broadcast only — denylist rebroadcast
/// must not consume encrypted peer slots).
pub(crate) fn broadcast_bssid_denylist(
    softap: Option<[u8; 6]>,
    home_gate: &Arc<Mutex<Option<[u8; 6]>>>,
    manual_gate: &Arc<RwLock<HashSet<[u8; 6]>>>,
    peer_if: wifi_interface_t,
) -> Result<()> {
    let home_now = home_gate.lock().ok().and_then(|g| *g);
    let manual = manual_gate
        .read()
        .map_err(|_| anyhow!("denylist manual lock poison"))?;
    let merged = config::merged_wigle_denylist_mac_vec(softap, home_now, &manual);
    gateway_downlink::broadcast_fleet_list(
        FleetList::BssidDenylist(&merged),
        peer_if,
        "control BSSID denylist",
    )
}

/// Push current `detection_disable_wifi_mask` to sniffers via `ControlCmd::Ping`.
pub(crate) fn broadcast_detection_disable_wifi_mask(
    sniffers: &Arc<Mutex<HashSet<[u8; 6]>>>,
    seq: &Arc<AtomicU32>,
    peer_if: wifi_interface_t,
    control_boot_id: u32,
) -> Result<()> {
    let builder = fleet_cmd_builder_from_plan();
    let broadcast_only = true;
    broadcast_cmd(
        sniffers,
        seq,
        ControlCmd::Ping,
        None,
        peer_if,
        control_boot_id,
        broadcast_only,
        &builder,
        None,
        0,
    )
}

pub(crate) fn send_bssid_denylist_unicast(
    mac: [u8; 6],
    merged: &[[u8; 6]],
    peer_if: wifi_interface_t,
) -> Result<()> {
    gateway_downlink::send_fleet_list_unicast(
        mac,
        FleetList::BssidDenylist(merged),
        peer_if,
        "espnow send BSSID denylist unicast",
    )
}

/// Push manual WiGLE SSID denylist to sniffers (broadcast only).
pub(crate) fn broadcast_ssid_denylist(
    manual_gate: &Arc<RwLock<HashSet<String>>>,
    peer_if: wifi_interface_t,
) -> Result<()> {
    let manual = manual_gate
        .read()
        .map_err(|_| anyhow!("ssid denylist manual lock poison"))?;
    let mut ssids: Vec<String> = manual.iter().cloned().collect();
    ssids.sort();
    gateway_downlink::broadcast_fleet_list(
        FleetList::SsidDenylist(&ssids),
        peer_if,
        "control SSID denylist",
    )
}

pub(crate) fn send_ssid_denylist_unicast(
    mac: [u8; 6],
    ssids: &[String],
    peer_if: wifi_interface_t,
) -> Result<()> {
    gateway_downlink::send_fleet_list_unicast(
        mac,
        FleetList::SsidDenylist(ssids),
        peer_if,
        "espnow send SSID denylist unicast",
    )
}

/// Push operator SSID watchlist to sniffers (beacon match → alert).
pub(crate) fn broadcast_ssid_watchlist(
    watch_gate: &Arc<RwLock<HashSet<String>>>,
    peer_if: wifi_interface_t,
) -> Result<()> {
    let manual = watch_gate
        .read()
        .map_err(|_| anyhow!("ssid watchlist lock poison"))?;
    let mut ssids: Vec<String> = manual.iter().cloned().collect();
    ssids.sort();
    gateway_downlink::broadcast_fleet_list(
        FleetList::SsidWatchlist(&ssids),
        peer_if,
        "control SSID watchlist",
    )
}

pub(crate) fn send_ssid_watchlist_unicast(
    mac: [u8; 6],
    ssids: &[String],
    peer_if: wifi_interface_t,
) -> Result<()> {
    gateway_downlink::send_fleet_list_unicast(
        mac,
        FleetList::SsidWatchlist(ssids),
        peer_if,
        "espnow send SSID watchlist unicast",
    )
}

pub(crate) fn send_set_channel_schedule_unicast(
    mac: [u8; 6],
    sniffer_id: u32,
    sched: &channel_plan::NodeSchedule,
    cmd_seq: &AtomicU32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    gps_time_seq: Option<u32>,
) {
    let band_wire = sched.band.map(|b| b.as_wire()).unwrap_or(0);
    let pinned_wire = if sched.pinned { sched.channels[0] } else { 0 };
    stats::set_sniffer_plan_assignment(
        sniffer_id,
        band_wire,
        pinned_wire,
        sched.chan_count,
        &sched.channels,
    );

    let cmd_seq_now = cmd_seq.fetch_add(1, Ordering::Relaxed);
    let mut cmd = ControlCmdV1 {
        cmd: ControlCmd::SetChannelSchedule,
        cmd_seq: cmd_seq_now,
        control_channel: CONTROL_AP_CHANNEL,
        dwell_ms: sched.dwell_ms,
        chan_count: sched.chan_count,
        channels: sched.channels,
        desired_role: 0,
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
    if let Err(e) = send_cmd_unicast(mac, cmd, peer_if) {
        warn!("control: SetChannelSchedule unicast to {sniffer_id}: {e:#}");
    }
}

/// Post-dump grace: push current channel plan to one Wi-Fi sniffer.
pub(crate) fn unicast_channel_schedule_for_sniffer(
    mac: [u8; 6],
    sniffer_id: u32,
    cmd_seq: &AtomicU32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    gps_time_seq: Option<u32>,
) {
    if !stats::scan_allowed() {
        return;
    }
    let now_ms = stats::now_ms();
    let mut active_buf = Vec::new();
    fill_active_wifi_sniffers(&mut active_buf, now_ms);
    let mut sig_vec: Vec<u32> = active_buf.iter().map(|(id, _)| *id).collect();
    sig_vec.sort_unstable();
    sig_vec.dedup();
    if !sig_vec.contains(&sniffer_id) {
        return;
    }

    let cached = super::channel_plan_cache::get_or_build(&sig_vec);
    let Some(sched) = cached.schedules.get(&sniffer_id) else {
        return;
    };
    send_set_channel_schedule_unicast(
        mac,
        sniffer_id,
        sched,
        cmd_seq,
        control_boot_id,
        peer_if,
        gps_time_seq,
    );
}

/// Post-dump encrypted downlink for `mac` (pending SetRole + SetChannelSchedule).
pub(crate) fn flush_post_dump_downlink(
    mac: [u8; 6],
    cmd_seq: &AtomicU32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    time_set_seq: Option<u32>,
) {
    let Some(sniffer_id) = super::espnow_rx::sniffer_id_for_mac(&mac) else {
        return;
    };
    let now_ms = stats::now_ms();
    if let Some((_, desired_role)) = stats::pending_role_for(sniffer_id) {
        let cmd_seq_now = cmd_seq.fetch_add(1, Ordering::Relaxed);
        if send_setrole_unicast(
            mac,
            desired_role,
            cmd_seq_now,
            control_boot_id,
            peer_if,
            time_set_seq,
        )
        .is_ok()
        {
            stats::mark_setrole_sent(sniffer_id, now_ms);
        }
    }
    unicast_channel_schedule_for_sniffer(
        mac,
        sniffer_id,
        cmd_seq,
        control_boot_id,
        peer_if,
        time_set_seq,
    );
}

/// Post-adopt / re-hello unicast: `ControlWelcome` + Start/Stop + denylists + pending SetRole.
pub(crate) fn send_sniffer_hello_welcome(
    peer: [u8; 6],
    sniffer_id: u32,
    fleet_slot: u8,
    control_boot_id: u32,
    cmd_seq: &AtomicU32,
    time_set_seq: u32,
    own_management_ap_bssid: Option<[u8; 6]>,
    wigle_home: &Arc<Mutex<Option<[u8; 6]>>>,
    wigle_manual: &Arc<RwLock<HashSet<[u8; 6]>>>,
    wigle_ssids_manual: &Arc<RwLock<HashSet<String>>>,
    ssid_watchlist: &Arc<RwLock<HashSet<String>>>,
    peer_if: wifi_interface_t,
) -> Result<()> {
    let want_scan = stats::scan_allowed();
    let target_ble = stats::pending_role_for(sniffer_id)
        .map(|(_, r)| r == 1)
        .unwrap_or(false)
        || stats::role_for_sniffer_id(sniffer_id) == Some(1);
    let (bootstrap_channels, bootstrap_cc) = channel_plan::current_plan().bootstrap_bucket();
    let plan_dwell = channel_plan::current_plan().dwell_ms;
    let send_wifi_schedule = want_scan && !target_ble;
    let schedule = if send_wifi_schedule {
        Some((bootstrap_channels, bootstrap_cc))
    } else {
        None
    };

    let ctx = WelcomeDownlinkCtx {
        cmd_seq,
        control_boot_id,
        peer_if,
        gps_time_seq: Some(time_set_seq),
    };
    let hooks = ControlWelcomeHooks {
        time_set_seq,
        peer_if,
        own_management_ap_bssid,
        wigle_home,
        wigle_manual,
        wigle_ssids_manual,
        ssid_watchlist,
    };
    let result = run_welcome_bundle(
        &hooks, &ctx, peer, sniffer_id, fleet_slot, want_scan, schedule, plan_dwell,
    );
    if result.ok {
        Ok(())
    } else {
        Err(anyhow!(
            "hello welcome failed at stage {:?}",
            result.failed_stage
        ))
    }
}

/// Control NVS / WiGLE policy sources for [`WelcomeDownlinkHooks`].
struct ControlWelcomeHooks<'a> {
    time_set_seq: u32,
    peer_if: wifi_interface_t,
    own_management_ap_bssid: Option<[u8; 6]>,
    wigle_home: &'a Arc<Mutex<Option<[u8; 6]>>>,
    wigle_manual: &'a Arc<RwLock<HashSet<[u8; 6]>>>,
    wigle_ssids_manual: &'a Arc<RwLock<HashSet<String>>>,
    ssid_watchlist: &'a Arc<RwLock<HashSet<String>>>,
}

impl WelcomeDownlinkHooks for ControlWelcomeHooks<'_> {
    fn ensure_peer(&self, mac: [u8; 6]) -> Result<()> {
        ensure_sniffer_peer_for_unicast(mac, self.peer_if, "espnow peer sniffer (hello welcome)")
    }

    fn send_control_welcome(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        fleet_slot: u8,
        want_scan: bool,
        schedule: Option<([u8; proto::CHANNELS_MAX], u8)>,
        dwell_ms: u16,
    ) -> Result<()> {
        let welcome = build_welcome(WelcomeConfig {
            control_boot_id: ctx.control_boot_id,
            control_channel: CONTROL_AP_CHANNEL,
            scan_allowed: want_scan,
            dwell_ms,
            schedule,
            detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
            wardrive_suppressed: stats::wardrive_suppressed(),
            fleet_slot,
            detection_disable_ble_mask: stats::detection_disable_ble_mask(),
            remote_id_disable_mask: stats::remote_id_disable_mask(),
            ble_active_scan: stats::ble_active_scan(),
        });
        espnow::send(mac, &proto::pack_control_welcome(welcome)).map_err(|e| {
            stats::HELLO_TX_WELCOME_FAIL.fetch_add(1, Ordering::Relaxed);
            anyhow!("hello path ControlWelcome send: {e:#}")
        })
    }

    fn send_start_stop(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        want_scan: bool,
        schedule: Option<([u8; proto::CHANNELS_MAX], u8)>,
        dwell_ms: u16,
    ) -> Result<u32> {
        let cmd_seq_now = ctx.cmd_seq.fetch_add(1, Ordering::Relaxed);
        let mut cmd = build_start_stop(StartStopConfig {
            cmd_seq: cmd_seq_now,
            control_channel: CONTROL_AP_CHANNEL,
            scan_allowed: want_scan,
            dwell_ms,
            schedule,
            desired_role: 0,
            control_boot_id: ctx.control_boot_id,
            detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
            wardrive_suppressed: stats::wardrive_suppressed(),
            time_set_seq: 0,
            ctrl_unix_ms: 0,
            detection_disable_ble_mask: stats::detection_disable_ble_mask(),
            remote_id_disable_mask: stats::remote_id_disable_mask(),
            ble_active_scan: stats::ble_active_scan(),
            peer_slots_free: 0,
            peer_join_waiters: 0,
        });
        attach_gps_time(&mut cmd, self.time_set_seq);
        send_cmd_unicast(mac, cmd, ctx.peer_if)?;
        Ok(cmd_seq_now)
    }

    fn send_channel_schedule(
        &self,
        _ctx: &WelcomeDownlinkCtx<'_>,
        _mac: [u8; 6],
        _sniffer_id: u32,
    ) -> Result<()> {
        Ok(())
    }

    fn send_bssid_denylist(&self, ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()> {
        let home_now = self.wigle_home.lock().ok().and_then(|g| *g);
        let Ok(manual) = self.wigle_manual.read() else {
            return Ok(());
        };
        let merged =
            config::merged_wigle_denylist_mac_vec(self.own_management_ap_bssid, home_now, &manual);
        send_bssid_denylist_unicast(mac, &merged, ctx.peer_if).map_err(|e| {
            stats::HELLO_TX_DENYLIST_FAIL.fetch_add(1, Ordering::Relaxed);
            warn!("control: hello path BSSID denylist unicast: {e:#}");
            e
        })
    }

    fn send_ssid_denylist(&self, ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()> {
        let Ok(manual) = self.wigle_ssids_manual.read() else {
            return Ok(());
        };
        let mut ssids: Vec<String> = manual.iter().cloned().collect();
        ssids.sort();
        send_ssid_denylist_unicast(mac, &ssids, ctx.peer_if).map_err(|e| {
            stats::HELLO_TX_DENYLIST_FAIL.fetch_add(1, Ordering::Relaxed);
            warn!("control: hello path SSID denylist unicast: {e:#}");
            e
        })
    }

    fn send_ssid_watchlist(&self, ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()> {
        let Ok(watch) = self.ssid_watchlist.read() else {
            return Ok(());
        };
        let mut ssids: Vec<String> = watch.iter().cloned().collect();
        ssids.sort();
        send_ssid_watchlist_unicast(mac, &ssids, ctx.peer_if).map_err(|e| {
            stats::HELLO_TX_DENYLIST_FAIL.fetch_add(1, Ordering::Relaxed);
            warn!("control: hello path SSID watchlist unicast: {e:#}");
            e
        })
    }

    fn send_set_role_if_pending(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        sniffer_id: u32,
    ) -> Result<()> {
        let Some((_, desired_role)) = stats::pending_role_for(sniffer_id) else {
            return Ok(());
        };
        let cmd_seq_role = ctx.cmd_seq.fetch_add(1, Ordering::Relaxed);
        let now_ms = stats::now_ms();
        if let Err(e) = send_setrole_unicast(
            mac,
            desired_role,
            cmd_seq_role,
            ctx.control_boot_id,
            ctx.peer_if,
            ctx.gps_time_seq,
        ) {
            warn!("control: hello path SetRole unicast: {e:#}");
        } else {
            stats::mark_setrole_sent(sniffer_id, now_ms);
        }
        Ok(())
    }
}

/// Clear the NVS fleet roster and unicast `ReleaseBinding` to every adopted sniffer.
pub(crate) fn unbind_all_fleet(
    nvs: &esp_idf_svc::nvs::EspNvsPartition<esp_idf_svc::nvs::NvsDefault>,
    peer_if: wifi_interface_t,
) -> Result<()> {
    use crate::fleet_roster;
    use log::info;

    let removed = fleet_roster::with_roster_mut(|r| r.clear_all()).unwrap_or_default();
    if removed.is_empty() {
        info!("control: fleet unbind: roster already empty");
        return Ok(());
    }
    let macs: Vec<[u8; 6]> = removed.iter().map(|(_, mac)| *mac).collect();
    let sent = release_bindings_for_macs(&macs, peer_if, RELEASE_BINDING_RETRIES);
    for (id, _) in &removed {
        stats::set_adoption_removed(*id);
    }
    fleet_roster::save_roster(nvs)?;
    info!(
        "control: fleet unbind release_binding {sent}/{} MACs",
        macs.len()
    );
    Ok(())
}
