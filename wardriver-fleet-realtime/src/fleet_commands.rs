//! Pure wolfPACK command and list payload construction.

use anyhow::Result;
use wardriver_proto::{self as proto, ControlCmd, ControlCmdV1, ControlWelcomeV1, CHANNELS_MAX};

/// Fleet policy list payload accepted by broadcast and unicast downlink helpers.
#[derive(Clone, Copy, Debug)]
pub enum FleetList<'a> {
    BssidDenylist(&'a [[u8; 6]]),
    SsidDenylist(&'a [String]),
    SsidWatchlist(&'a [String]),
}

impl FleetList<'_> {
    pub fn pack(self) -> Result<Vec<u8>> {
        match self {
            Self::BssidDenylist(macs) => proto::pack_bssid_denylist(macs),
            Self::SsidDenylist(ssids) => proto::pack_ssid_denylist(ssids),
            Self::SsidWatchlist(ssids) => proto::pack_ssid_watchlist(ssids),
        }
    }
}

/// Inputs shared by control and gateway welcome construction.
#[derive(Clone, Copy, Debug)]
pub struct WelcomeConfig {
    pub control_boot_id: u32,
    pub control_channel: u8,
    pub scan_allowed: bool,
    pub dwell_ms: u16,
    pub schedule: Option<([u8; CHANNELS_MAX], u8)>,
    pub detection_disable_wifi_mask: u32,
    pub wardrive_suppressed: bool,
    pub fleet_slot: u8,
    pub detection_disable_ble_mask: u32,
    pub remote_id_disable_mask: u32,
    pub ble_active_scan: bool,
}

#[must_use]
pub fn build_welcome(config: WelcomeConfig) -> ControlWelcomeV1 {
    let (channels, chan_count) = config.schedule.unwrap_or(([0u8; CHANNELS_MAX], 0));
    ControlWelcomeV1 {
        control_boot_id: config.control_boot_id,
        control_channel: config.control_channel,
        desired_scanning: u8::from(config.scan_allowed),
        dwell_ms: config.dwell_ms,
        chan_count: if config.scan_allowed { chan_count } else { 0 },
        channels: if config.scan_allowed {
            channels
        } else {
            [0u8; CHANNELS_MAX]
        },
        proto_min: proto::PROTO_VERSION,
        proto_max: proto::PROTO_VERSION,
        detection_disable_wifi_mask: config.detection_disable_wifi_mask,
        wardrive_suppressed: u8::from(config.wardrive_suppressed),
        fleet_slot: config.fleet_slot,
        detection_disable_ble_mask: config.detection_disable_ble_mask,
        remote_id_disable_mask: config.remote_id_disable_mask,
        ble_active_scan: u8::from(config.ble_active_scan),
    }
}

/// Inputs shared by control and gateway welcome follow-on Start/Stop construction.
#[derive(Clone, Copy, Debug)]
pub struct StartStopConfig {
    pub cmd_seq: u32,
    pub control_boot_id: u32,
    pub control_channel: u8,
    pub scan_allowed: bool,
    pub dwell_ms: u16,
    pub schedule: Option<([u8; CHANNELS_MAX], u8)>,
    pub desired_role: u8,
    pub detection_disable_wifi_mask: u32,
    pub wardrive_suppressed: bool,
    pub time_set_seq: u32,
    pub ctrl_unix_ms: u64,
    pub detection_disable_ble_mask: u32,
    pub remote_id_disable_mask: u32,
    pub ble_active_scan: bool,
    pub peer_slots_free: u8,
    pub peer_join_waiters: u8,
}

#[must_use]
pub fn build_start_stop(config: StartStopConfig) -> ControlCmdV1 {
    let (channels, chan_count) = config.schedule.unwrap_or(([0u8; CHANNELS_MAX], 0));
    ControlCmdV1 {
        cmd: if config.scan_allowed {
            ControlCmd::StartScanning
        } else {
            ControlCmd::StopScanning
        },
        cmd_seq: config.cmd_seq,
        control_channel: config.control_channel,
        dwell_ms: config.dwell_ms,
        chan_count: if config.scan_allowed { chan_count } else { 0 },
        channels: if config.scan_allowed {
            channels
        } else {
            [0u8; CHANNELS_MAX]
        },
        desired_role: config.desired_role,
        control_boot_id: config.control_boot_id,
        detection_disable_wifi_mask: config.detection_disable_wifi_mask,
        wardrive_suppressed: u8::from(config.wardrive_suppressed),
        time_set_seq: config.time_set_seq,
        ctrl_unix_ms: config.ctrl_unix_ms,
        detection_disable_ble_mask: config.detection_disable_ble_mask,
        remote_id_disable_mask: config.remote_id_disable_mask,
        ble_active_scan: u8::from(config.ble_active_scan),
        desired_scanning: 0,
        peer_slots_free: config.peer_slots_free,
        peer_join_waiters: config.peer_join_waiters,
    }
}

/// Fleet keepalive cadence shared by control and gateway loops.
pub const PERIODIC_PING_INTERVAL_MS: u64 = 300;

#[must_use]
pub fn periodic_ping_due(last_ping_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(last_ping_ms) >= PERIODIC_PING_INTERVAL_MS
}

/// Suppress a queue-hint ping only when the regular ping is due within 50 ms.
#[must_use]
pub fn queue_hint_ping_should_send(last_ping_ms: u64, now_ms: u64) -> bool {
    let elapsed = now_ms.saturating_sub(last_ping_ms);
    elapsed >= PERIODIC_PING_INTERVAL_MS || PERIODIC_PING_INTERVAL_MS.saturating_sub(elapsed) > 50
}
