//! Cached [`wardriver_core::channel_plan::effective_from_plan`] for active Wi-Fi sniffers.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::channel_plan::{self, NodeSchedule};
use wardriver_fleet_realtime::channel_plan_runtime::PlanRuntimeCache;

static CACHE: Mutex<PlanRuntimeCache> = Mutex::new(PlanRuntimeCache::new());

/// Snapshot of per-sniffer schedules (clone on cache hit; cheaper than replanning).
#[derive(Clone, Debug)]
pub(crate) struct CachedEffective {
    pub schedules: BTreeMap<u32, NodeSchedule>,
}

/// Return cached schedules when `(revision, active_sig)` match; otherwise rebuild once.
pub(crate) fn get_or_build(active_ids: &[u32]) -> CachedEffective {
    let rev = channel_plan::revision();
    let plan = channel_plan::current_plan();
    let effective = CACHE
        .lock()
        .map(|mut guard| guard.get_effective(&plan, rev, active_ids))
        .unwrap_or_else(|_| {
            let mut plan_snapshot = plan.clone();
            plan_snapshot.prune_unknown_sniffers(active_ids);
            channel_plan::effective_from_plan(&plan_snapshot, active_ids)
        });
    CachedEffective {
        schedules: effective.schedules,
    }
}
