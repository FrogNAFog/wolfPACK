//! Sniffer channel schedule signature (host-testable; no esp-idf).

use wardriver_proto::{ControlCmdV1, CHANNELS_MAX};

/// Hop cadence guardrail: during sweeping we want ≥ 2 hops/sec instantaneous.
pub const SNIFFER_SWEEP_MAX_DWELL_MS: u16 = 500;

/// Schedule fields hashed for cheap change detection (mirrors firmware `ChannelSchedule`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnifferScheduleSig {
    pub control_channel: u8,
    pub dwell_ms: u16,
    pub chan_count: u8,
    pub channels: [u8; CHANNELS_MAX],
}

impl SnifferScheduleSig {
    pub fn from_control_cmd(cmd: &ControlCmdV1) -> Self {
        Self {
            control_channel: cmd.control_channel.max(1),
            dwell_ms: cmd.dwell_ms.max(50).min(SNIFFER_SWEEP_MAX_DWELL_MS),
            chan_count: cmd.chan_count.min(CHANNELS_MAX as u8),
            channels: cmd.channels,
        }
    }
}

/// Small stable hash (FNV-1a) over schedule fields.
pub fn schedule_sig(s: &SnifferScheduleSig) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in [
        s.control_channel,
        (s.dwell_ms & 0xff) as u8,
        (s.dwell_ms >> 8) as u8,
        s.chan_count,
    ] {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    for b in s.channels {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
