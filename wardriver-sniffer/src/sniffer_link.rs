//! Control link: discovery hello backoff and periodic ESP-NOW watchdog.

use std::sync::atomic::{AtomicBool, Ordering};

use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use log::{info, warn};

use crate::sniffer_config::SnifferConfig;
use crate::sniffer_wifi::sniffer_boot_id;
use wardriver_core::liveness::{link_dead_due, link_reboot_due, SNIFFER_UPLINK_LIVENESS_MS};
use wardriver_proto::{self as proto, SnifferHelloV1, SnifferRole};
use wardriver_shared::espnow;

/// Poll cadence for link-dead / reboot checks (not the liveness budget itself).
const WATCHDOG_INTERVAL_MS: u64 = 5_000;

const HELLO_BACKOFF_MS: &[u64] = &[250, 500, 1_000, 2_000, 5_000];

static LINK_RESET_PENDING: AtomicBool = AtomicBool::new(false);

/// True after [`enter_link_dead`] until Welcome or `PeerJoinAck` clears recovery.
#[must_use]
pub(crate) fn link_reset_pending() -> bool {
    LINK_RESET_PENDING.load(Ordering::Relaxed)
}

pub(crate) fn clear_link_reset_pending() {
    LINK_RESET_PENDING.store(false, Ordering::Relaxed);
}

/// Boot-time 5 GHz capability flags (wired into [`SnifferHelloV1`]).
#[derive(Clone, Copy)]
pub(crate) struct SnifferLinkCaps {
    pub five_ghz_capable: u8,
    /// PROTO cleanup candidate: mirrors `five_ghz_capable` on the wire until next PROTO bump.
    pub dfs_capable: u8,
}

/// Returned when the watchdog enters unified link-dead recovery.
pub(crate) enum WatchdogAction {
    LinkDead,
}

/// SnifferLinkState hello backoff and link-watchdog state for the main sniffer loop.
pub(crate) struct SnifferLinkState {
    hello_backoff_idx: usize,
    next_hello_due_ms: u64,
    last_watchdog_tick_ms: u64,
    /// Snapshot of combined ESP-NOW RX + HTTP contact liveness from the prior tick.
    last_control_liveness_snapshot_ms: Option<u64>,
    /// Set on first watchdog tick where RX snapshot is unchanged; cleared when RX advances.
    link_dead_armed_at_ms: Option<u64>,
    /// Set when [`Self::reset_fast_join`] runs (link-dead recovery).
    reconnect_grace_bump: bool,
}

impl SnifferLinkState {
    pub fn new() -> Self {
        Self {
            hello_backoff_idx: 1,
            next_hello_due_ms: wardriver_stats::now_ms().saturating_add(HELLO_BACKOFF_MS[0]),
            last_watchdog_tick_ms: 0,
            last_control_liveness_snapshot_ms: None,
            link_dead_armed_at_ms: None,
            reconnect_grace_bump: false,
        }
    }

    /// Restart fast-join hello broadcasts (link-dead recovery).
    pub fn reset_fast_join(&mut self, now_ms: u64) {
        self.hello_backoff_idx = 0;
        self.next_hello_due_ms = now_ms;
        self.reconnect_grace_bump = true;
    }

    /// True once after each [`Self::reset_fast_join`]; clears the flag.
    pub fn take_reconnect_grace_bump(&mut self) -> bool {
        core::mem::replace(&mut self.reconnect_grace_bump, false)
    }

    #[deny(dead_code)]
    /// Discovery loop: broadcast Hello until control is established.
    pub fn tick_discovery_hello(
        &mut self,
        now_ms: u64,
        control_established: bool,
        sniffer_id: u32,
        cfg: &SnifferConfig,
        caps: SnifferLinkCaps,
    ) {
        if !control_established && now_ms >= self.next_hello_due_ms {
            send_sniffer_hello(sniffer_id, cfg, caps);
            let delay = HELLO_BACKOFF_MS
                .get(self.hello_backoff_idx)
                .copied()
                .unwrap_or(HELLO_BACKOFF_MS[HELLO_BACKOFF_MS.len() - 1]);
            self.next_hello_due_ms = now_ms.saturating_add(delay);
            self.hello_backoff_idx = (self.hello_backoff_idx + 1).min(HELLO_BACKOFF_MS.len());
        }
    }

    /// 5 s tick: heartbeat ACK gate + unified link-dead / reboot watchdog.
    ///
    /// `last_control_liveness_ms` is the latest ESP-NOW RX from control.
    pub fn tick_watchdog(
        &mut self,
        now_ms: u64,
        control_channel: u8,
        last_control_liveness_ms: Option<u64>,
        control_established: bool,
        pending_ack: &mut bool,
        _nvs: &EspNvsPartition<NvsDefault>,
        cfg: &mut SnifferConfig,
        control_mac: &mut Option<[u8; 6]>,
        last_control_boot_id: &mut Option<u32>,
        control_established_flag: &mut bool,
    ) -> Option<WatchdogAction> {
        if now_ms.saturating_sub(self.last_watchdog_tick_ms) < WATCHDOG_INTERVAL_MS {
            return None;
        }
        self.last_watchdog_tick_ms = now_ms;

        *pending_ack = *pending_ack || control_established;

        if last_control_liveness_ms.is_none() {
            self.link_dead_armed_at_ms = None;
            self.last_control_liveness_snapshot_ms = None;
            return None;
        }

        // 5 s-cadence diagnostic: how long ago did the sniffer last hear control
        // (gateway ping)? If this grows past the 20 s budget while the gateway
        // keeps transmitting, pings are being dropped on the radio.
        if let Some(seen) = last_control_liveness_ms {
            info!(
                "sniffer: link alive_age={} ms established={}",
                now_ms.saturating_sub(seen),
                control_established
            );
        }

        let heard_since_last_tick = match (
            self.last_control_liveness_snapshot_ms,
            last_control_liveness_ms,
        ) {
            (Some(prev), Some(cur)) if cur != prev => true,
            (None, Some(_)) => true,
            _ => false,
        };
        if heard_since_last_tick {
            self.link_dead_armed_at_ms = None;
        } else if self.link_dead_armed_at_ms.is_none() {
            self.link_dead_armed_at_ms = Some(now_ms);
        }
        self.last_control_liveness_snapshot_ms = last_control_liveness_ms;

        if link_reboot_due(self.link_dead_armed_at_ms, now_ms) {
            wardriver_stats::SNIFFER_WATCHDOG_REBOOT.fetch_add(1, Ordering::Relaxed);
            warn!(
                "sniffer: watchdog: no control liveness for >= {} ms; rebooting",
                SNIFFER_UPLINK_LIVENESS_MS.saturating_mul(2)
            );
            unsafe {
                esp_idf_sys::esp_restart();
            }
        }

        if link_dead_due(self.link_dead_armed_at_ms, now_ms) && !link_reset_pending() {
            self.enter_link_dead(
                now_ms,
                control_channel,
                cfg,
                control_mac,
                last_control_boot_id,
                control_established_flag,
            );
            return Some(WatchdogAction::LinkDead);
        }

        None
    }

    fn enter_link_dead(
        &mut self,
        now_ms: u64,
        control_channel: u8,
        cfg: &SnifferConfig,
        control_mac: &mut Option<[u8; 6]>,
        last_control_boot_id: &mut Option<u32>,
        control_established_flag: &mut bool,
    ) {
        if link_reset_pending() {
            return;
        }
        LINK_RESET_PENDING.store(true, Ordering::Relaxed);
        wardriver_stats::SNIFFER_LINK_DEAD.fetch_add(1, Ordering::Relaxed);
        warn!(
            "sniffer: link dead (no control liveness for >= {} ms); ESP-NOW recovery",
            SNIFFER_UPLINK_LIVENESS_MS
        );
        crate::sniffer_dispatch::clear_peer_ready();
        *control_established_flag = false;
        *last_control_boot_id = None;
        // Keep NVS fleet binding; drop session only.
        *control_mac = cfg.bound_control_mac;
        let _ = espnow::set_channel(control_channel);
        if let Err(e) = espnow::reinit_stack() {
            warn!("sniffer: link-dead esp_now reinit_stack: {e:#}");
        }
        let _ =
            espnow::add_broadcast_peer(control_channel, esp_idf_sys::wifi_interface_t_WIFI_IF_STA);
        if let Some(mac) = cfg.bound_control_mac.or(*control_mac) {
            let _ = espnow::add_peer(
                mac,
                control_channel,
                esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
            );
        }
        self.reset_fast_join(now_ms);
        self.link_dead_armed_at_ms = None;
    }
}

/// Local fleet unbind (BOOT button or `ReleaseBinding`).
pub(crate) fn apply_local_fleet_unbind(
    cfg: &mut SnifferConfig,
    nvs: &EspNvsPartition<NvsDefault>,
    control_mac: &mut Option<[u8; 6]>,
    control_established: &mut bool,
    last_control_boot_id: &mut Option<u32>,
    fleet_slot: &mut u8,
) {
    if !cfg.is_fleet_bound() && cfg.control_mac.is_none() {
        return;
    }
    info!("sniffer: fleet binding released locally");
    cfg.clear_fleet_binding();
    let _ = cfg.save(nvs);
    *control_mac = None;
    *control_established = false;
    *last_control_boot_id = None;
    *fleet_slot = 0;
    crate::sniffer_espnow::FLEET_FAST_JOIN_RESET.store(true, Ordering::Relaxed);
}

/// One immediate hello on boot so control can respond without waiting for periodic Ping.
pub(crate) fn send_boot_hello(sniffer_id: u32, cfg: &SnifferConfig, caps: SnifferLinkCaps) {
    send_sniffer_hello(sniffer_id, cfg, caps);
}

fn send_sniffer_hello(sniffer_id: u32, cfg: &SnifferConfig, caps: SnifferLinkCaps) {
    wardriver_stats::SNIFFER_HELLO_TX.fetch_add(1, Ordering::Relaxed);
    let role = SnifferRole::from_wire(cfg.role);
    let payload = proto::pack_sniffer_hello(SnifferHelloV1 {
        sniffer_id,
        sniffer_boot_id: sniffer_boot_id(),
        role,
        ble_capable: if cfg!(feature = "ble") { 1 } else { 0 },
        five_ghz_capable: caps.five_ghz_capable,
        dfs_capable: caps.dfs_capable,
        proto_min: proto::PROTO_VERSION,
        proto_max: proto::PROTO_VERSION,
    });
    let _ = espnow::send(espnow::BROADCAST_MAC, &payload);
}
