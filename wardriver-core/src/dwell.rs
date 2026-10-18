//! Dwell and sweep wall-clock budgets (schedule-aware control silence).

use crate::channel_plan::{ChannelPlan, MAX_DWELL_MS, MIN_DWELL_MS};
use wardriver_proto::{
    BLE_SCAN_WINDOW_MS, SNIFFER_MAX_OFF_CONTROL_MS, SNIFFER_SWEEP_SET_CHANNEL_BUDGET_MS_PER_HOP,
};

/// Floor for control-RX silence before the sniffer drops `control_established`.
pub const CONTROL_SILENCE_TIMEOUT_FLOOR_MS: u64 = 15_000;

/// Headroom added on top of expected sweep wall time for the control-RX watchdog.
pub const CONTROL_SILENCE_TIMEOUT_MARGIN_MS: u64 = 7_500;

/// Per-channel dwell and hop count for timeout / sweep budgeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DwellAndSchedule {
    pub dwell_ms: u16,
    pub channel_count: usize,
    pub role_is_ble: bool,
    /// Measured WiFi `set_channel` average (µs); 0 uses proto budget only.
    pub set_channel_us_avg: u32,
}

impl DwellAndSchedule {
    /// Clamp operator / plan dwell to the channel-plan legal range.
    #[must_use]
    pub fn clamp_dwell_ms(dwell_ms: u16) -> u16 {
        dwell_ms.clamp(MIN_DWELL_MS, MAX_DWELL_MS)
    }

    /// Per-channel dwell from the active operator channel plan (fleet command authority).
    #[must_use]
    pub fn dwell_ms_from_plan(plan: &ChannelPlan) -> u16 {
        Self::clamp_dwell_ms(plan.dwell_ms)
    }

    #[must_use]
    pub fn listen_wall_ms(&self) -> u64 {
        let n = self.channel_count.max(1) as u64;
        (self.dwell_ms as u64).saturating_mul(n)
    }

    /// Expected wall-clock capture phase (listen sum + hop overhead or BLE slack).
    #[must_use]
    pub fn sweep_wall_ms(&self) -> u64 {
        if self.role_is_ble {
            return BLE_SCAN_WINDOW_MS.saturating_add(100);
        }
        let listen_ms = self.listen_wall_ms();
        let n_ch = self.channel_count.max(1) as u64;
        let avg_ms_per_hop = if self.set_channel_us_avg > 0 {
            ((self.set_channel_us_avg as u64) + 999) / 1000
        } else {
            0
        };
        let budget_per_hop = SNIFFER_SWEEP_SET_CHANNEL_BUDGET_MS_PER_HOP.max(avg_ms_per_hop);
        let hop_overhead_ms = budget_per_hop.saturating_mul(n_ch);
        // Small buckets finish one rotation well before the off-control cap; budget to the cap
        // so silence timeouts and GPS retention match resumed multi-rotation Phase A.
        let capture_ms = if listen_ms < SNIFFER_MAX_OFF_CONTROL_MS {
            SNIFFER_MAX_OFF_CONTROL_MS
        } else {
            listen_ms
        };
        capture_ms.saturating_add(hop_overhead_ms)
    }

    /// Control-RX silence budget: `max(floor, sweep + margin) + extra_allowance_ms`.
    #[must_use]
    pub fn control_silence_timeout_ms(&self, extra_allowance_ms: u64) -> u64 {
        let expected_total = self
            .sweep_wall_ms()
            .saturating_add(CONTROL_SILENCE_TIMEOUT_MARGIN_MS);
        CONTROL_SILENCE_TIMEOUT_FLOOR_MS
            .max(expected_total)
            .saturating_add(extra_allowance_ms)
    }

    /// Mid-sweep cap for time off the control channel ([`SNIFFER_MAX_OFF_CONTROL_MS`]).
    #[must_use]
    pub const fn max_off_control_ms() -> u64 {
        SNIFFER_MAX_OFF_CONTROL_MS
    }
}
