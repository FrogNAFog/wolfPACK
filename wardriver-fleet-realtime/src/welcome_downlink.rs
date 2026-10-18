//! ESP-NOW welcome bundle downlink: shared stage sequence with node-specific hooks.

use std::sync::atomic::AtomicU32;

use anyhow::Result;
use esp_idf_sys::wifi_interface_t;
use wardriver_proto::CHANNELS_MAX;

use crate::welcome_bundle::{
    run_welcome_bundle as run_stages, WelcomeAttemptResult, WelcomeBundleHooks,
};

/// Shared downlink context passed to every welcome hook (cmd seq, boot id, peer if, GPS piggyback).
pub struct WelcomeDownlinkCtx<'a> {
    pub cmd_seq: &'a AtomicU32,
    pub control_boot_id: u32,
    pub peer_if: wifi_interface_t,
    pub gps_time_seq: Option<u32>,
}

/// Node-specific welcome downlink hooks (gateway `PolicySyncV1` vs control NVS gates).
pub trait WelcomeDownlinkHooks {
    /// Stage 1: install encrypted ESP-NOW peer before any unicast.
    fn ensure_peer(&self, mac: [u8; 6]) -> Result<()>;
    /// Stage 2: unicast `ControlWelcome` (boot id, scan policy, fleet slot, dwell, schedule).
    fn send_control_welcome(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        fleet_slot: u8,
        want_scan: bool,
        schedule: Option<([u8; CHANNELS_MAX], u8)>,
        dwell_ms: u16,
    ) -> Result<()>;
    /// Stage 3: unicast Start/Stop; returns `cmd_seq` echoed in `SnifferAck`.
    fn send_start_stop(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        want_scan: bool,
        schedule: Option<([u8; CHANNELS_MAX], u8)>,
        dwell_ms: u16,
    ) -> Result<u32>;
    /// Stage 4: optional per-sniffer channel schedule (dedicated plan / bootstrap).
    fn send_channel_schedule(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        sniffer_id: u32,
    ) -> Result<()>;
    /// Stage 5: BSSID denylist unicast (no-op inside hook when empty).
    fn send_bssid_denylist(&self, ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()>;
    /// Stage 6: SSID denylist unicast (no-op inside hook when empty).
    fn send_ssid_denylist(&self, ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()>;
    /// Stage 7: SSID watchlist unicast (no-op inside hook when empty).
    fn send_ssid_watchlist(&self, ctx: &WelcomeDownlinkCtx<'_>, mac: [u8; 6]) -> Result<()>;
    /// Stage 8: pending `SetRole` when roster/policy queued a role change.
    fn send_set_role_if_pending(
        &self,
        ctx: &WelcomeDownlinkCtx<'_>,
        mac: [u8; 6],
        sniffer_id: u32,
    ) -> Result<()>;
}

struct DownlinkAdapter<'a, H: WelcomeDownlinkHooks> {
    hooks: &'a H,
    ctx: &'a WelcomeDownlinkCtx<'a>,
    mac: [u8; 6],
    sniffer_id: u32,
    fleet_slot: u8,
    want_scan: bool,
    schedule: Option<([u8; CHANNELS_MAX], u8)>,
    dwell_ms: u16,
}

impl<H: WelcomeDownlinkHooks> WelcomeBundleHooks for DownlinkAdapter<'_, H> {
    fn ensure_peer(&self) -> Result<()> {
        self.hooks.ensure_peer(self.mac)
    }

    fn send_control_welcome(&self) -> Result<()> {
        self.hooks.send_control_welcome(
            self.ctx,
            self.mac,
            self.fleet_slot,
            self.want_scan,
            self.schedule,
            self.dwell_ms,
        )
    }

    fn send_start_stop(&self) -> Result<u32> {
        self.hooks.send_start_stop(
            self.ctx,
            self.mac,
            self.want_scan,
            self.schedule,
            self.dwell_ms,
        )
    }

    fn send_channel_schedule(&self) -> Result<()> {
        self.hooks
            .send_channel_schedule(self.ctx, self.mac, self.sniffer_id)
    }

    fn send_bssid_denylist(&self) -> Result<()> {
        self.hooks.send_bssid_denylist(self.ctx, self.mac)
    }

    fn send_ssid_denylist(&self) -> Result<()> {
        self.hooks.send_ssid_denylist(self.ctx, self.mac)
    }

    fn send_ssid_watchlist(&self) -> Result<()> {
        self.hooks.send_ssid_watchlist(self.ctx, self.mac)
    }

    fn send_set_role_if_pending(&self) -> Result<()> {
        self.hooks
            .send_set_role_if_pending(self.ctx, self.mac, self.sniffer_id)
    }
}

/// Run the canonical welcome bundle downlink sequence via node-specific hooks.
#[must_use]
pub fn run_welcome_bundle(
    hooks: &impl WelcomeDownlinkHooks,
    ctx: &WelcomeDownlinkCtx<'_>,
    mac: [u8; 6],
    sniffer_id: u32,
    fleet_slot: u8,
    want_scan: bool,
    schedule: Option<([u8; CHANNELS_MAX], u8)>,
    dwell_ms: u16,
) -> WelcomeAttemptResult {
    let adapter = DownlinkAdapter {
        hooks,
        ctx,
        mac,
        sniffer_id,
        fleet_slot,
        want_scan,
        schedule,
        dwell_ms,
    };
    run_stages(&adapter)
}
