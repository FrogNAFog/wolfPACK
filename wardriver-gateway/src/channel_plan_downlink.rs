//! Phone `ChannelPlanSyncV1` RAM mirror and per-sniffer ESP-NOW schedule downlink.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use esp_idf_sys::wifi_interface_t;
use log::warn;
use wardriver_core::channel_plan::{
    self, BandAssignmentMode, ChannelPlan, NodeSchedule, PerSnifferChannel,
};
use wardriver_fleet_realtime::channel_plan_runtime::{
    fill_active_wifi_ids, hash_u32_list, push_all_schedules, rebalance_due, PlanRuntimeCache,
    SchedulePush, REBALANCE_MIN_INTERVAL_MS,
};
use wardriver_fleet_realtime::gateway_downlink::{attach_gps_time, send_cmd_unicast};
use wardriver_fleet_realtime::CONTROL_AP_CHANNEL;
use wardriver_packlink::{
    band_mode, channel_plan_sync_ack_error, channel_plan_sync_ack_result, ChannelPlanSyncAckV1,
    ChannelPlanSyncV1, SnifferStatusPlanExtV1,
};
use wardriver_proto::{ControlCmd, ControlCmdV1};
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::policy_downlink::GatewayFleetState;

struct ChannelPlanState {
    dedicated: bool,
    revision: u32,
    plan: ChannelPlan,
    hop_cycle_ms: u32,
    unassigned_count: u16,
    runtime_cache: PlanRuntimeCache,
}

static LAST_REBALANCE_MS: OnceLock<AtomicU32> = OnceLock::new();

fn last_rebalance_ms() -> &'static AtomicU32 {
    LAST_REBALANCE_MS.get_or_init(|| AtomicU32::new(0))
}

static CHANNEL_PLAN: OnceLock<Mutex<ChannelPlanState>> = OnceLock::new();

fn plan_state() -> &'static Mutex<ChannelPlanState> {
    CHANNEL_PLAN.get_or_init(|| {
        Mutex::new(ChannelPlanState {
            dedicated: false,
            revision: 0,
            plan: ChannelPlan::default(),
            hop_cycle_ms: 0,
            unassigned_count: 0,
            runtime_cache: PlanRuntimeCache::new(),
        })
    })
}

/// PackLink `GatewayInfo` capability tail: dedicated channel-plan sync.
#[must_use]
pub fn gateway_capabilities() -> u8 {
    wardriver_packlink::gateway_capabilities::CHANNEL_PLAN_SYNC
}

/// True when a phone `ChannelPlanSync` has been applied (per-sniffer schedules active).
#[must_use]
pub fn dedicated_plan_active() -> bool {
    plan_state().lock().map(|g| g.dedicated).unwrap_or(false)
}

fn sync_to_plan(sync: &ChannelPlanSyncV1) -> ChannelPlan {
    let mut per_sniffer = Vec::new();
    for (id, ch) in &sync.pins {
        per_sniffer.push(PerSnifferChannel {
            sniffer_id: *id,
            pinned_channel: if *ch == 0 { None } else { Some(*ch) },
        });
    }
    ChannelPlan {
        include_2_4_ghz: sync.include_2_4_ghz,
        include_5_ghz: sync.include_5_ghz,
        include_dfs: sync.include_dfs,
        dwell_ms: sync.dwell_ms,
        channels_2_4: sync.channels_2_4.clone(),
        channels_5: sync.channels_5.clone(),
        channels_5_dfs: sync.channels_5_dfs.clone(),
        sniffers_2_4: sync.sniffers_2_4.clone(),
        sniffers_5: sync.sniffers_5.clone(),
        per_sniffer,
        band_mode: if sync.band_mode == band_mode::MANUAL {
            BandAssignmentMode::Manual
        } else {
            BandAssignmentMode::AutoBalance
        },
    }
}

fn validate_channel_plan(plan: &ChannelPlan) -> Result<()> {
    if plan.master_scan_pool().is_empty() {
        return Err(anyhow!("at least one enabled band must have channels"));
    }
    Ok(())
}

fn get_effective(plan: &ChannelPlan, revision: u32, active_ids: &[u32]) -> channel_plan::Effective {
    if let Ok(mut guard) = plan_state().lock() {
        let effective = guard
            .runtime_cache
            .get_effective(plan, revision, active_ids);
        guard.hop_cycle_ms = effective.hop_cycle_ms;
        guard.unassigned_count = effective.unassigned.len().min(u16::MAX as usize) as u16;
        return effective;
    }
    let mut plan_snapshot = plan.clone();
    plan_snapshot.prune_unknown_sniffers(active_ids);
    channel_plan::effective_from_plan(&plan_snapshot, active_ids)
}

struct GatewaySchedulePush<'a> {
    fleet: &'a GatewayFleetState,
    peer_if: wifi_interface_t,
}

impl SchedulePush for GatewaySchedulePush<'_> {
    fn push_schedule(
        &self,
        sniffer_id: u32,
        mac: [u8; 6],
        sched: &NodeSchedule,
        gps_time_seq: Option<u32>,
    ) {
        send_set_channel_schedule_unicast(
            mac,
            sniffer_id,
            sched,
            &self.fleet.cmd_seq,
            self.fleet.control_boot_id.load(Ordering::Relaxed),
            self.peer_if,
            gps_time_seq,
        );
        task_wdt::reset_if_subscribed();
    }
}

fn send_set_channel_schedule_unicast(
    mac: [u8; 6],
    sniffer_id: u32,
    sched: &NodeSchedule,
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
        warn!("gateway: SetChannelSchedule unicast to {sniffer_id}: {e:#}");
    }
}

/// Apply phone channel plan and fan out per-sniffer schedules.
pub fn apply_channel_plan_sync(
    fleet: &GatewayFleetState,
    sync: ChannelPlanSyncV1,
    plan_seq: u32,
) -> ChannelPlanSyncAckV1 {
    let mut plan = sync_to_plan(&sync);
    plan.normalize_channels();
    plan.clamp_dwell();

    let ack_err = |error_code: u8| -> ChannelPlanSyncAckV1 {
        ChannelPlanSyncAckV1 {
            plan_seq,
            revision: sync.revision,
            result: channel_plan_sync_ack_result::REJECTED,
            error_code,
            hop_cycle_ms: 0,
            unassigned_count: 0,
        }
    };

    if let Err(e) = validate_channel_plan(&plan) {
        warn!("gateway: channel_plan validation: {e:#}");
        return ack_err(channel_plan_sync_ack_error::VALIDATION);
    }

    let mut active_ids = Vec::new();
    fill_active_wifi_ids(&mut active_ids, stats::now_ms());
    let effective = get_effective(&plan, sync.revision, &active_ids);

    if let Ok(mut guard) = plan_state().lock() {
        guard.dedicated = true;
        guard.revision = sync.revision;
        guard.plan = plan;
        guard.hop_cycle_ms = effective.hop_cycle_ms;
        guard.unassigned_count = effective.unassigned.len().min(u16::MAX as usize) as u16;
    }

    task_wdt::reset_if_subscribed();
    push_gateway_schedules(
        fleet,
        &effective.schedules,
        Some(stats::time_set_seq_for_wire()),
    );
    if let Ok(mut guard) = plan_state().lock() {
        if guard.dedicated && guard.revision == sync.revision {
            let active_sig = hash_u32_list(&active_ids);
            guard.runtime_cache.record_push(sync.revision, active_sig);
            last_rebalance_ms().store(
                stats::now_ms().min(u32::MAX as u64) as u32,
                Ordering::Relaxed,
            );
        }
    }

    ChannelPlanSyncAckV1 {
        plan_seq,
        revision: sync.revision,
        result: channel_plan_sync_ack_result::OK,
        error_code: channel_plan_sync_ack_error::NONE,
        hop_cycle_ms: effective.hop_cycle_ms,
        unassigned_count: effective.unassigned.len().min(u16::MAX as usize) as u16,
    }
}

/// Recompute and push schedules when the active Wi-Fi fleet changes.
pub fn rebalance_if_dedicated(fleet: &GatewayFleetState, gps_time_seq: Option<u32>) {
    if !stats::scan_allowed() {
        return;
    }
    let now_ms = stats::now_ms();
    let last = u64::from(last_rebalance_ms().load(Ordering::Relaxed));
    let mut active_ids = Vec::new();
    fill_active_wifi_ids(&mut active_ids, now_ms);
    let active_sig = hash_u32_list(&active_ids);
    let (plan, revision) = {
        let Ok(guard) = plan_state().lock() else {
            return;
        };
        if !guard.dedicated {
            return;
        }
        if !rebalance_due(
            &guard.runtime_cache,
            guard.revision,
            active_sig,
            now_ms.saturating_sub(last),
            REBALANCE_MIN_INTERVAL_MS,
        ) {
            return;
        }
        (guard.plan.clone(), guard.revision)
    };
    let effective = get_effective(&plan, revision, &active_ids);
    push_gateway_schedules(fleet, &effective.schedules, gps_time_seq);
    if let Ok(mut guard) = plan_state().lock() {
        if guard.dedicated && guard.revision == revision {
            guard.runtime_cache.record_push(revision, active_sig);
            last_rebalance_ms().store(now_ms.min(u32::MAX as u64) as u32, Ordering::Relaxed);
        }
    }
}

fn push_gateway_schedules(
    fleet: &GatewayFleetState,
    schedules: &BTreeMap<u32, NodeSchedule>,
    gps_time_seq: Option<u32>,
) {
    let push = GatewaySchedulePush {
        fleet,
        peer_if: wardriver_fleet_realtime::ESPNOW_PEER_IF,
    };
    push_all_schedules(&push, schedules, stats::mac_for_sniffer_id, gps_time_seq);
}

/// Post-dump / hello: unicast one sniffer's effective schedule when dedicated plan is active.
pub fn unicast_schedule_for_sniffer(
    fleet: &GatewayFleetState,
    mac: [u8; 6],
    sniffer_id: u32,
    peer_if: wifi_interface_t,
    gps_time_seq: Option<u32>,
) {
    if !dedicated_plan_active() || !stats::scan_allowed() {
        return;
    }
    if stats::role_for_sniffer_id(sniffer_id) == Some(1) {
        return;
    }
    let (plan, revision) = {
        let Ok(guard) = plan_state().lock() else {
            return;
        };
        if !guard.dedicated {
            return;
        }
        (guard.plan.clone(), guard.revision)
    };
    let mut active_ids = Vec::new();
    fill_active_wifi_ids(&mut active_ids, stats::now_ms());
    if !active_ids.contains(&sniffer_id) {
        return;
    }
    let effective = get_effective(&plan, revision, &active_ids);
    let Some(sched) = effective.schedules.get(&sniffer_id) else {
        stats::clear_sniffer_plan_assignment(sniffer_id);
        return;
    };
    send_set_channel_schedule_unicast(
        mac,
        sniffer_id,
        sched,
        &fleet.cmd_seq,
        fleet.control_boot_id.load(Ordering::Relaxed),
        peer_if,
        gps_time_seq,
    );
}

/// Optional `SnifferStatus` plan-extension row from stats slot.
#[must_use]
pub fn plan_ext_for_slot(
    five_ghz_capable: u8,
    assigned_band: u8,
    pinned_channel: u8,
    assigned_chan_count: u8,
    assigned_channels: &[u8; wardriver_proto::CHANNELS_MAX],
) -> Option<SnifferStatusPlanExtV1> {
    if !dedicated_plan_active() && assigned_chan_count == 0 && assigned_band == 0 {
        return None;
    }
    let mut preview = [0u8; 4];
    let n = assigned_chan_count.min(4) as usize;
    preview[..n].copy_from_slice(&assigned_channels[..n]);
    Some(SnifferStatusPlanExtV1 {
        five_ghz_capable,
        assigned_band,
        pinned_channel,
        assigned_chan_count,
        assigned_channels_preview: preview,
    })
}
