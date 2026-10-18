//! Phone `PolicySyncV1` RAM mirror and ESP-NOW downlink fan-out.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::gateway_adoption;
use anyhow::Result;
use esp_idf_sys::wifi_interface_t;
use log::{info, warn};
use wardriver_fleet_realtime::fleet_commands::{
    build_start_stop, build_welcome, FleetList, StartStopConfig, WelcomeConfig,
};
use wardriver_fleet_realtime::fleet_tick;
use wardriver_fleet_realtime::gateway_downlink::{
    self, broadcast_cmd, broadcast_fleet_list, ensure_sniffer_peer_for_unicast, send_cmd_unicast,
    send_fleet_list_unicast, send_setrole_unicast, FleetCommandBuilder, GpsTimeHooks,
};
use wardriver_fleet_realtime::peer_pool;
use wardriver_fleet_realtime::time_sync_rtt::{self, WallClockHooks};
use wardriver_fleet_realtime::welcome_bundle::WelcomeAttemptResult;
use wardriver_fleet_realtime::welcome_downlink::{
    run_welcome_bundle, WelcomeDownlinkCtx, WelcomeDownlinkHooks,
};
use wardriver_fleet_realtime::{CONTROL_AP_CHANNEL, ESPNOW_PEER_IF};
use wardriver_packlink::{
    policy_has_fleet_config, roster_role_for_mac, PolicySyncV1, RosterEntryV1,
};
use wardriver_proto::{self as proto, ControlCmd, ControlCmdV1};
use wardriver_shared::espnow;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::adoption_retry;
use crate::channel_plan_downlink;
use crate::gateway_softap;

/// Shared gateway fleet handles (policy RAM, cmd seq, peer pool hooks).
pub struct GatewayFleetState {
    pub policy: Arc<Mutex<PolicySyncV1>>,
    pub cmd_seq: AtomicU32,
    pub control_boot_id: AtomicU32,
    pub sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    pub scanning_allowed: AtomicBool,
    /// Set when RAM policy mirror holds roster or scan intent ([`policy_has_fleet_config`]).
    pub policy_applied: AtomicBool,
}

/// True when wolfPACK broadcast `Ping` should run (gateway).
#[must_use]
pub fn fleet_ping_broadcast_ready(policy_applied: bool) -> bool {
    fleet_tick::fleet_ping_broadcast_ready(policy_applied)
}

/// True when the gateway RAM policy mirror holds wardrive roster or scan intent.
#[must_use]
pub fn policy_mirror_has_fleet_config(state: &GatewayFleetState) -> bool {
    state
        .policy
        .lock()
        .ok()
        .is_some_and(|p| policy_has_fleet_config(&p))
}

impl GatewayFleetState {
    #[must_use]
    pub fn new(control_boot_id: u32) -> Self {
        Self {
            policy: Arc::new(Mutex::new(empty_policy())),
            cmd_seq: AtomicU32::new(1),
            control_boot_id: AtomicU32::new(control_boot_id),
            sniffers: Arc::new(Mutex::new(HashSet::new())),
            scanning_allowed: AtomicBool::new(false),
            policy_applied: AtomicBool::new(false),
        }
    }

    pub fn register_hooks(&self) {
        gateway_downlink::register_gps_time_hooks(GpsTimeHooks {
            wall_trusted: phone_wall_trusted,
            unix_now_ms: phone_unix_now_ms,
        });
        time_sync_rtt::register_wall_clock(WallClockHooks {
            trusted: phone_wall_trusted,
            trusted_since_ms: phone_wall_trusted_since_ms,
            unix_now_ms: phone_unix_now_ms,
        });
    }

    pub fn cmd_builder(&self) -> FleetCommandBuilder {
        let dwell = self.policy.lock().map(|p| p.dwell_ms).unwrap_or(0);
        FleetCommandBuilder::new(dwell)
    }
}

#[must_use]
pub fn empty_policy() -> PolicySyncV1 {
    PolicySyncV1 {
        scan_allowed: false,
        wardrive_suppressed: false,
        geofence_enabled: false,
        geofence_lat_min: 0.0,
        geofence_lat_max: 0.0,
        geofence_lon_min: 0.0,
        geofence_lon_max: 0.0,
        denylist_bssids: Vec::new(),
        denylist_ssids: Vec::new(),
        roster: Vec::new(),
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; proto::CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    }
}

/// Apply phone policy to RAM stats and fan out wolfPACK downlink (Ping + denylists + schedule).
pub fn apply_policy_sync(state: &GatewayFleetState, policy: PolicySyncV1) -> Result<()> {
    let prev_roster: Vec<RosterEntryV1> = state
        .policy
        .lock()
        .map(|g| g.roster.clone())
        .unwrap_or_default();

    stats::set_scan_allowed(policy.scan_allowed);
    stats::set_wardrive_suppressed(policy.wardrive_suppressed);
    state
        .scanning_allowed
        .store(policy.scan_allowed, Ordering::Release);
    task_wdt::reset_if_subscribed();

    if let Ok(mut g) = state.policy.lock() {
        *g = policy.clone();
    }
    if policy_has_fleet_config(&policy) {
        state.policy_applied.store(true, Ordering::Release);
    } else {
        state.policy_applied.store(false, Ordering::Release);
    }
    crate::policy_store::stage_save(&policy);
    task_wdt::reset_if_subscribed();

    send_release_binding_for_removed_roster(state, &prev_roster, &policy.roster);
    task_wdt::reset_if_subscribed();

    let now_ms = stats::now_ms();
    adoption_retry::sync_roster(&policy.roster, &prev_roster, now_ms);
    task_wdt::reset_if_subscribed();

    let merged_bssids = merged_bssid_denylist(&policy.denylist_bssids);
    broadcast_bssid_denylist(&merged_bssids)?;
    task_wdt::reset_if_subscribed();
    broadcast_ssid_denylist(&policy.denylist_ssids)?;
    task_wdt::reset_if_subscribed();

    // Detection policy: flock masks in RAM stats + ESP-NOW downlink (mirrors control preferences).
    stats::set_detection_disable_masks(
        policy.detection_disable_wifi_mask,
        policy.detection_disable_ble_mask,
    );
    stats::set_remote_id_disable_mask(policy.remote_id_disable_mask);
    stats::set_ble_active_scan(policy.ble_active_scan);
    broadcast_ssid_watchlist(&policy.ssid_watch)?;
    task_wdt::reset_if_subscribed();

    for entry in &policy.roster {
        let desired = entry.role.min(1);
        let current = stats::role_for_sniffer_id(entry.sniffer_id);
        if current == Some(desired) {
            task_wdt::reset_if_subscribed();
            continue;
        }
        stats::enqueue_role_change(entry.sniffer_id, entry.mac, desired);
        if desired == 1 {
            stats::enqueue_deferred_welcome(entry.sniffer_id, entry.mac, entry.fleet_slot, true);
        }
        task_wdt::reset_if_subscribed();
    }

    let builder = FleetCommandBuilder::new(policy.dwell_ms);
    let desired_scanning = u8::from(policy.scan_allowed);
    task_wdt::reset_if_subscribed();
    broadcast_cmd(
        &state.sniffers,
        &state.cmd_seq,
        ControlCmd::Ping,
        None,
        ESPNOW_PEER_IF,
        state.control_boot_id.load(Ordering::Relaxed),
        true,
        &builder,
        Some(stats::time_set_seq_for_wire()),
        desired_scanning,
    )
}

/// After gateway boot or NVS policy restore: force welcome for every rostered sniffer.
pub fn enqueue_boot_welcomes(state: &GatewayFleetState) {
    let roster = state
        .policy
        .lock()
        .map(|g| g.roster.clone())
        .unwrap_or_default();
    for entry in roster {
        stats::enqueue_deferred_welcome(entry.sniffer_id, entry.mac, entry.fleet_slot, true);
    }
}

/// Prepend gateway management SoftAP BSSID, then phone manual list (deduped).
fn merged_bssid_denylist(manual: &[[u8; 6]]) -> Vec<[u8; 6]> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(softap) = gateway_softap::read_softap_mac() {
        if seen.insert(softap) {
            out.push(softap);
        }
    }
    for m in manual {
        if seen.insert(*m) {
            out.push(*m);
        }
    }
    out
}

fn broadcast_bssid_denylist(macs: &[[u8; 6]]) -> Result<()> {
    broadcast_fleet_list(
        FleetList::BssidDenylist(macs),
        ESPNOW_PEER_IF,
        "gateway BSSID denylist",
    )
}

fn broadcast_ssid_denylist(ssids: &[String]) -> Result<()> {
    broadcast_fleet_list(
        FleetList::SsidDenylist(ssids),
        ESPNOW_PEER_IF,
        "gateway SSID denylist",
    )
}

/// Push operator SSID watchlist to sniffers (beacon match → alert).
fn broadcast_ssid_watchlist(ssids: &[String]) -> Result<()> {
    broadcast_fleet_list(
        FleetList::SsidWatchlist(ssids),
        ESPNOW_PEER_IF,
        "gateway SSID watchlist",
    )
}

/// Roster hello hook: unicast welcome + start/stop + policy denylists.
///
/// Returns structured attempt result including Start/Stop `cmd_seq` for adoption FSM.
pub fn hello_welcome(
    state: &GatewayFleetState,
    peer: [u8; 6],
    fleet_slot: u8,
    _rebooted: bool,
) -> WelcomeAttemptResult {
    let policy = state
        .policy
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_else(empty_policy);
    let want_scan = policy.scan_allowed;
    let dwell = policy.dwell_ms;
    let schedule = if channel_plan_downlink::dedicated_plan_active() {
        None
    } else if policy.chan_count > 0 {
        Some((policy.channels, policy.chan_count))
    } else {
        None
    };
    let sniffer_id = stats::sniffer_id_for_mac(&peer).unwrap_or(0);
    let ctx = WelcomeDownlinkCtx {
        cmd_seq: &state.cmd_seq,
        control_boot_id: state.control_boot_id.load(Ordering::Relaxed),
        peer_if: ESPNOW_PEER_IF,
        gps_time_seq: Some(stats::time_set_seq_for_wire()),
    };
    let hooks = GatewayWelcomeHooks { state, policy };
    run_welcome_bundle(
        &hooks, &ctx, peer, sniffer_id, fleet_slot, want_scan, schedule, dwell,
    )
}

/// Gateway policy sources for [`WelcomeDownlinkHooks`].
struct GatewayWelcomeHooks<'a> {
    state: &'a GatewayFleetState,
    policy: PolicySyncV1,
}

impl WelcomeDownlinkHooks for GatewayWelcomeHooks<'_> {
    fn ensure_peer(&self, mac: [u8; 6]) -> Result<()> {
        ensure_sniffer_peer_for_unicast(mac, ESPNOW_PEER_IF, "gateway hello welcome").map_err(|e| {
            warn!("gateway: hello welcome peer: {e:#}");
            e
        })
    }

    fn send_control_welcome(
        &self,
        _ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        fleet_slot: u8,
        want_scan: bool,
        schedule: Option<([u8; proto::CHANNELS_MAX], u8)>,
        dwell_ms: u16,
    ) -> Result<()> {
        let welcome = build_welcome(WelcomeConfig {
            control_boot_id: self.state.control_boot_id.load(Ordering::Relaxed),
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
            warn!("gateway: ControlWelcome send: {e:#}");
            anyhow::anyhow!("gateway: ControlWelcome send: {e:#}")
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
        let desired_role = roster_role_for_mac(&self.policy, &mac);
        let cmd_seq_now = ctx.cmd_seq.fetch_add(1, Ordering::Relaxed);
        let cmd = build_start_stop(StartStopConfig {
            cmd_seq: cmd_seq_now,
            control_channel: CONTROL_AP_CHANNEL,
            scan_allowed: want_scan,
            dwell_ms,
            schedule,
            desired_role,
            control_boot_id: ctx.control_boot_id,
            detection_disable_wifi_mask: stats::detection_disable_wifi_mask(),
            wardrive_suppressed: stats::wardrive_suppressed(),
            time_set_seq: stats::time_set_seq_for_wire(),
            ctrl_unix_ms: phone_unix_now_ms().unwrap_or(0),
            detection_disable_ble_mask: stats::detection_disable_ble_mask(),
            peer_slots_free: peer_pool::peer_slots_free_estimate(),
            peer_join_waiters: peer_pool::join_queue_waiter_count(),
            remote_id_disable_mask: stats::remote_id_disable_mask(),
            ble_active_scan: stats::ble_active_scan(),
        });
        send_cmd_unicast(mac, cmd, ctx.peer_if).map_err(|e| {
            warn!("gateway: hello Start/Stop: {e:#}");
            anyhow::anyhow!("gateway: hello Start/Stop: {e:#}")
        })?;
        Ok(cmd_seq_now)
    }

    fn send_channel_schedule(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        sniffer_id: u32,
    ) -> Result<()> {
        if sniffer_id == 0 || stats::sniffer_id_for_mac(&mac).is_none() {
            return Ok(());
        }
        channel_plan_downlink::unicast_schedule_for_sniffer(
            self.state,
            mac,
            sniffer_id,
            ctx.peer_if,
            ctx.gps_time_seq,
        );
        Ok(())
    }

    fn send_bssid_denylist(&self, _ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()> {
        let merged = merged_bssid_denylist(&self.policy.denylist_bssids);
        if merged.is_empty() {
            return Ok(());
        }
        send_fleet_list_unicast(
            mac,
            FleetList::BssidDenylist(&merged),
            ESPNOW_PEER_IF,
            "gateway hello BSSID denylist",
        )
    }

    fn send_ssid_denylist(&self, _ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()> {
        if self.policy.denylist_ssids.is_empty() {
            return Ok(());
        }
        send_fleet_list_unicast(
            mac,
            FleetList::SsidDenylist(&self.policy.denylist_ssids),
            ESPNOW_PEER_IF,
            "gateway hello SSID denylist",
        )
    }

    fn send_ssid_watchlist(&self, _ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()> {
        if self.policy.ssid_watch.is_empty() {
            return Ok(());
        }
        send_fleet_list_unicast(
            mac,
            FleetList::SsidWatchlist(&self.policy.ssid_watch),
            ESPNOW_PEER_IF,
            "gateway hello SSID watchlist",
        )
    }

    fn send_set_role_if_pending(
        &self,
        _ctx: &WelcomeDownlinkCtx<'_>,
        _mac: [u8; 6],
        _sniffer_id: u32,
    ) -> Result<()> {
        Ok(())
    }
}

/// Phone wall-clock anchor (full `u64` ms). ESP32 `std` has no `AtomicU64`.
#[derive(Clone, Copy, Debug, Default)]
struct PhoneWallClock {
    trusted: bool,
    wall_ms: u64,
    trusted_since_mono_ms: u64,
}

static PHONE_WALL: OnceLock<Mutex<PhoneWallClock>> = OnceLock::new();

fn phone_wall() -> &'static Mutex<PhoneWallClock> {
    PHONE_WALL.get_or_init(|| Mutex::new(PhoneWallClock::default()))
}

/// Phone wrote `time_sync`; latch wall clock for ESP-NOW piggyback + post-dump RTT.
pub fn note_phone_time_sync(unix_ms: u64, time_set_seq: u32) {
    if unix_ms == 0 {
        return;
    }
    let now_mono = stats::now_ms();
    if let Ok(mut clock) = phone_wall().lock() {
        clock.trusted = true;
        clock.wall_ms = unix_ms;
        clock.trusted_since_mono_ms = now_mono;
    }
    stats::TIME_SET_SEQ.store(time_set_seq, Ordering::Relaxed);
}

fn phone_wall_snapshot() -> Option<PhoneWallClock> {
    let clock = phone_wall().lock().ok()?;
    if clock.trusted {
        Some(*clock)
    } else {
        None
    }
}

fn phone_wall_trusted() -> bool {
    phone_wall_snapshot().is_some()
}

fn phone_wall_trusted_since_ms() -> u64 {
    phone_wall_snapshot()
        .map(|clock| clock.trusted_since_mono_ms)
        .unwrap_or(0)
}

fn phone_unix_now_ms() -> Option<u64> {
    let clock = phone_wall_snapshot()?;
    let elapsed = stats::now_ms().saturating_sub(clock.trusted_since_mono_ms);
    Some(clock.wall_ms.saturating_add(elapsed))
}

/// Phone-synced wall clock for PackLink `EventNotify.unix_ms` (not boot-monotonic).
#[must_use]
pub fn gateway_phone_unix_ms_now() -> Option<u64> {
    phone_unix_now_ms()
}

/// Read current policy (clone).
#[must_use]
pub fn current_policy(state: &GatewayFleetState) -> PolicySyncV1 {
    state
        .policy
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| empty_policy())
}

/// Store policy from BLE write and fan out downlink.
pub fn on_policy_sync_write(state: &GatewayFleetState, data: &[u8]) -> Result<PolicySyncV1> {
    let policy = crate::softap_transport::on_policy_sync_write(data)?;
    apply_policy_sync(state, policy.clone())?;
    Ok(policy)
}

fn send_release_binding_for_removed_roster(
    _state: &GatewayFleetState,
    prev: &[RosterEntryV1],
    next: &[RosterEntryV1],
) {
    let next_ids: HashSet<u32> = next.iter().map(|e| e.sniffer_id).collect();
    let mut macs = Vec::new();
    for entry in prev {
        if next_ids.contains(&entry.sniffer_id) {
            continue;
        }
        stats::set_adoption_removed(entry.sniffer_id);
        gateway_adoption::cancel(entry.sniffer_id);
        macs.push(entry.mac);
    }
    if macs.is_empty() {
        return;
    }
    let sent = wardriver_fleet_realtime::gateway_downlink::release_bindings_for_macs(
        &macs,
        ESPNOW_PEER_IF,
        wardriver_fleet_realtime::gateway_downlink::RELEASE_BINDING_RETRIES,
    );
    if sent < macs.len() {
        warn!(
            "gateway: release_binding roster diff {sent}/{} MACs sent",
            macs.len()
        );
    }
}

pub use wardriver_fleet_realtime::gateway_downlink::{
    release_bindings_for_macs, send_release_binding_unicast, RELEASE_BINDING_RETRIES,
};

/// Clear RAM roster, release sniffers, and apply empty policy.
pub fn unbind_all_fleet(state: &GatewayFleetState) -> Result<()> {
    let macs: Vec<[u8; 6]> = state
        .policy
        .lock()
        .map(|p| p.roster.iter().map(|e| e.mac).collect())
        .unwrap_or_default();
    if !macs.is_empty() {
        let sent = release_bindings_for_macs(&macs, ESPNOW_PEER_IF, RELEASE_BINDING_RETRIES);
        info!(
            "gateway: fleet unbind release_binding {sent}/{} MACs",
            macs.len()
        );
    }
    apply_policy_sync(state, empty_policy())?;
    crate::softap_transport::notify_fleet_unbind_if_connected();
    Ok(())
}

/// Post-dump encrypted downlink for `mac` (pending SetRole + phone schedule).
pub fn gateway_post_dump_flush(
    mac: [u8; 6],
    cmd_seq: &AtomicU32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    time_set_seq: Option<u32>,
) {
    let Some(sniffer_id) = stats::sniffer_id_for_mac(&mac) else {
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
    unicast_channel_schedule_from_policy(
        mac,
        sniffer_id,
        cmd_seq,
        control_boot_id,
        peer_if,
        time_set_seq,
    );
}

fn unicast_channel_schedule_from_policy(
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
    let role = stats::role_for_sniffer_id(sniffer_id);
    if role == Some(1) {
        return;
    }
    if channel_plan_downlink::dedicated_plan_active() {
        let Some(fleet) = crate::run::fleet_state() else {
            return;
        };
        channel_plan_downlink::unicast_schedule_for_sniffer(
            fleet,
            mac,
            sniffer_id,
            peer_if,
            gps_time_seq,
        );
        return;
    }
    let Some(fleet) = crate::run::fleet_state() else {
        return;
    };
    let policy = current_policy(fleet);
    if policy.chan_count == 0 {
        return;
    }
    let cmd_seq_now = cmd_seq.fetch_add(1, Ordering::Relaxed);
    let mut cmd = ControlCmdV1 {
        cmd: ControlCmd::SetChannelSchedule,
        cmd_seq: cmd_seq_now,
        control_channel: CONTROL_AP_CHANNEL,
        dwell_ms: policy.dwell_ms,
        chan_count: policy.chan_count,
        channels: policy.channels,
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
        gateway_downlink::attach_gps_time(&mut cmd, seq);
    }
    if let Err(e) = send_cmd_unicast(mac, cmd, peer_if) {
        warn!("gateway: post-dump SetChannelSchedule sniffer_id={sniffer_id}: {e:#}");
    }
}
