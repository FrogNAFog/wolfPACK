//! Structured welcome bundle attempt result (gateway adoption diagnostics).

use anyhow::Result;

/// Stage that failed during a welcome bundle send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WelcomeStage {
    EnsurePeer,
    ControlWelcome,
    StartStop,
    ChannelSchedule,
    BssidDenylist,
    SsidDenylist,
    SsidWatchlist,
    SetRole,
}

/// Outcome of a welcome + follow-on downlink sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WelcomeAttemptResult {
    pub ok: bool,
    /// Start/Stop `cmd_seq` the sniffer echoes in `SnifferAck.ack_cmd_seq`.
    pub ack_cmd_seq: Option<u32>,
    pub failed_stage: Option<WelcomeStage>,
}

impl WelcomeAttemptResult {
    #[must_use]
    pub fn success(ack_cmd_seq: u32) -> Self {
        Self {
            ok: true,
            ack_cmd_seq: Some(ack_cmd_seq),
            failed_stage: None,
        }
    }

    #[must_use]
    pub fn failure(stage: WelcomeStage) -> Self {
        Self {
            ok: false,
            ack_cmd_seq: None,
            failed_stage: Some(stage),
        }
    }

    /// A later bundle stage failed after Start/Stop was successfully sent.
    #[must_use]
    pub fn failure_after_start_stop(stage: WelcomeStage, ack_cmd_seq: u32) -> Self {
        Self {
            ok: false,
            ack_cmd_seq: Some(ack_cmd_seq),
            failed_stage: Some(stage),
        }
    }
}

/// Canonical welcome bundle stage order (host-testable; mirrored by ESP downlink hooks).
pub const WELCOME_STAGE_ORDER: [WelcomeStage; 8] = [
    WelcomeStage::EnsurePeer,
    WelcomeStage::ControlWelcome,
    WelcomeStage::StartStop,
    WelcomeStage::ChannelSchedule,
    WelcomeStage::BssidDenylist,
    WelcomeStage::SsidDenylist,
    WelcomeStage::SsidWatchlist,
    WelcomeStage::SetRole,
];

/// Per-stage send hooks for the canonical welcome bundle sequence.
///
/// Node-specific policy (NVS vs `PolicySyncV1`) lives in the hook implementation,
/// not in [`run_welcome_bundle`].
pub trait WelcomeBundleHooks {
    /// Stage 1: install encrypted ESP-NOW peer before any unicast.
    fn ensure_peer(&self) -> Result<()>;
    /// Stage 2: unicast `ControlWelcome` (boot id, scan policy, fleet slot).
    fn send_control_welcome(&self) -> Result<()>;
    /// Stage 3: unicast Start/Stop; returns `cmd_seq` echoed in `SnifferAck`.
    fn send_start_stop(&self) -> Result<u32>;
    /// Stage 4: optional per-sniffer channel schedule (dedicated plan / bootstrap).
    fn send_channel_schedule(&self) -> Result<()>;
    /// Stage 5: BSSID denylist unicast (skip inside hook when empty).
    fn send_bssid_denylist(&self) -> Result<()>;
    /// Stage 6: SSID denylist unicast (skip inside hook when empty).
    fn send_ssid_denylist(&self) -> Result<()>;
    /// Stage 7: SSID watchlist unicast (skip inside hook when empty).
    fn send_ssid_watchlist(&self) -> Result<()>;
    /// Stage 8: pending `SetRole` when roster/policy queued a role change.
    fn send_set_role_if_pending(&self) -> Result<()>;
}

/// Run the canonical welcome bundle stages in [`WELCOME_STAGE_ORDER`].
#[must_use]
pub fn run_welcome_bundle(hooks: &impl WelcomeBundleHooks) -> WelcomeAttemptResult {
    // Stage 1 — peer slot required before encrypted unicasts.
    if hooks.ensure_peer().is_err() {
        return WelcomeAttemptResult::failure(WelcomeStage::EnsurePeer);
    }
    // Stage 2 — ControlWelcome advertises control boot id and scan policy.
    if hooks.send_control_welcome().is_err() {
        return WelcomeAttemptResult::failure(WelcomeStage::ControlWelcome);
    }
    // Stage 3 — Start/Stop is the adoption ACK anchor (`ack_cmd_seq`).
    let ack_cmd_seq = match hooks.send_start_stop() {
        Ok(seq) => seq,
        Err(_) => return WelcomeAttemptResult::failure(WelcomeStage::StartStop),
    };
    // Stages 4–8 — follow-on policy; failures retain Start/Stop seq for FSM.
    if hooks.send_channel_schedule().is_err() {
        return WelcomeAttemptResult::failure_after_start_stop(
            WelcomeStage::ChannelSchedule,
            ack_cmd_seq,
        );
    }
    if hooks.send_bssid_denylist().is_err() {
        return WelcomeAttemptResult::failure_after_start_stop(
            WelcomeStage::BssidDenylist,
            ack_cmd_seq,
        );
    }
    if hooks.send_ssid_denylist().is_err() {
        return WelcomeAttemptResult::failure_after_start_stop(
            WelcomeStage::SsidDenylist,
            ack_cmd_seq,
        );
    }
    if hooks.send_ssid_watchlist().is_err() {
        return WelcomeAttemptResult::failure_after_start_stop(
            WelcomeStage::SsidWatchlist,
            ack_cmd_seq,
        );
    }
    if hooks.send_set_role_if_pending().is_err() {
        return WelcomeAttemptResult::failure_after_start_stop(WelcomeStage::SetRole, ack_cmd_seq);
    }
    WelcomeAttemptResult::success(ack_cmd_seq)
}
