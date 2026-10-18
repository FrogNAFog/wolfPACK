//! Shared channel-plan cache, rebalance policy wrapper, and ESP schedule push helpers.
//!
//! Cache keys are `(revision, active_sig)` where `active_sig` is a FNV-1a hash of the
//! sorted active Wi-Fi sniffer id list. A revision or active-set change invalidates the
//! cached [`Effective`] and triggers a replan on the next `get_effective` call.
//!
//! Rebalance pushes are gated by [`rebalance_due`]: plan revision changes fire
//! immediately; active-set changes wait for [`REBALANCE_MIN_INTERVAL_MS`].

use wardriver_core::channel_plan::{self, ChannelPlan, Effective};

use crate::rebalance_policy::should_rebalance;

/// Minimum interval between dedicated-plan rebalance pushes (matches control grace).
pub const REBALANCE_MIN_INTERVAL_MS: u64 = 5_000;

/// FNV-1a 64-bit hash of a `u32` slice (little-endian bytes per element).
///
/// Used as `active_sig` so cache and rebalance logic can compare active Wi-Fi sets
/// without retaining the full id vector.
#[must_use]
pub fn hash_u32_list(v: &[u32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for x in v {
        for b in x.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}

/// RAM cache for [`effective_from_plan`] keyed by plan revision and active sniffer set.
///
/// Also tracks the last pushed `(revision, active_sig)` pair for rebalance decisions.
#[derive(Clone, Debug, Default)]
pub struct PlanRuntimeCache {
    revision: u32,
    active_sig: u64,
    effective: Option<Effective>,
    pub last_pushed_revision: Option<u32>,
    pub last_pushed_active_sig: Option<u64>,
}

impl PlanRuntimeCache {
    /// Empty cache with no pushed state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            revision: 0,
            active_sig: 0,
            effective: None,
            last_pushed_revision: None,
            last_pushed_active_sig: None,
        }
    }

    /// Return cached [`Effective`] when `(revision, active_sig)` match; otherwise rebuild once.
    ///
    /// `active_ids` should be sorted and deduplicated Wi-Fi-role sniffer ids.
    pub fn get_effective(
        &mut self,
        plan: &ChannelPlan,
        revision: u32,
        active_ids: &[u32],
    ) -> Effective {
        let active_sig = hash_u32_list(active_ids);
        if self.revision == revision && self.active_sig == active_sig {
            if let Some(ref effective) = self.effective {
                return effective.clone();
            }
        }

        let mut plan_snapshot = plan.clone();
        plan_snapshot.prune_unknown_sniffers(active_ids);
        let effective = channel_plan::effective_from_plan(&plan_snapshot, active_ids);
        self.revision = revision;
        self.active_sig = active_sig;
        self.effective = Some(effective.clone());
        effective
    }

    /// Record that schedules for `revision` and `active_sig` were dispatched.
    pub fn record_push(&mut self, revision: u32, active_sig: u64) {
        self.last_pushed_revision = Some(revision);
        self.last_pushed_active_sig = Some(active_sig);
    }
}

/// True when a dedicated plan push is due per [`should_rebalance`].
///
/// Plan revision mismatches rebalance immediately. Active-set mismatches wait until
/// `elapsed_since_push_ms >= min_interval_ms`.
#[must_use]
pub fn rebalance_due(
    cache: &PlanRuntimeCache,
    current_revision: u32,
    current_active_sig: u64,
    elapsed_since_push_ms: u64,
    min_interval_ms: u64,
) -> bool {
    should_rebalance(
        cache.last_pushed_revision,
        cache.last_pushed_active_sig,
        current_revision,
        current_active_sig,
        elapsed_since_push_ms,
        min_interval_ms,
    )
}

#[cfg(feature = "esp")]
pub use esp::*;

#[cfg(feature = "esp")]
mod esp {
    use std::collections::BTreeMap;

    use wardriver_core::channel_plan::NodeSchedule;
    use wardriver_core::liveness::SNIFFER_UPLINK_LIVENESS_MS;
    use wardriver_stats as stats;

    /// Node-specific unicast of a computed [`NodeSchedule`].
    pub trait SchedulePush {
        fn push_schedule(
            &self,
            sniffer_id: u32,
            mac: [u8; 6],
            sched: &NodeSchedule,
            gps_time_seq: Option<u32>,
        );
    }

    /// Fan out schedules to live sniffers; skips ids with no MAC from `active_mac_lookup`.
    pub fn push_all_schedules<P: SchedulePush>(
        push: &P,
        schedules: &BTreeMap<u32, NodeSchedule>,
        active_mac_lookup: impl Fn(u32) -> Option<[u8; 6]>,
        gps_time_seq: Option<u32>,
    ) {
        for (sniffer_id, sched) in schedules {
            let Some(mac) = active_mac_lookup(*sniffer_id) else {
                continue;
            };
            push.push_schedule(*sniffer_id, mac, sched, gps_time_seq);
        }
    }

    /// Fill `out` with sorted, deduplicated Wi-Fi-role sniffer ids seen within the uplink liveness window.
    pub fn fill_active_wifi_ids(out: &mut Vec<u32>, now_ms: u64) {
        out.clear();
        stats::for_each_sniffer_slot(|s| {
            if s.role != 0 {
                return;
            }
            if now_ms.saturating_sub(s.last_seen_ms) > SNIFFER_UPLINK_LIVENESS_MS {
                return;
            }
            out.push(s.sniffer_id);
        });
        out.sort_unstable();
        out.dedup();
    }
}
