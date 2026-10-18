//! Host coverage for [`PlanRuntimeCache`] and [`rebalance_due`].

use wardriver_core::channel_plan::{effective_from_plan, ChannelPlan};
use wardriver_fleet_realtime::channel_plan_runtime::{
    hash_u32_list, rebalance_due, PlanRuntimeCache, REBALANCE_MIN_INTERVAL_MS,
};

const ACTIVE_A: [u32; 3] = [10, 20, 30];
const ACTIVE_B: [u32; 3] = [10, 20, 31];

fn assert_schedules_match_cache_and_direct(
    cache: &mut PlanRuntimeCache,
    plan: &ChannelPlan,
    revision: u32,
    active_ids: &[u32],
) {
    let cached = cache.get_effective(plan, revision, active_ids);
    let mut plan_snapshot = plan.clone();
    plan_snapshot.prune_unknown_sniffers(active_ids);
    let direct = effective_from_plan(&plan_snapshot, active_ids);
    assert_eq!(cached.schedules, direct.schedules);
    assert_eq!(cached.hop_cycle_ms, direct.hop_cycle_ms);
    assert_eq!(cached.unassigned, direct.unassigned);
}

#[test]
fn cache_hit_avoids_rebuild_when_keys_unchanged() {
    let plan = ChannelPlan::default();
    let mut cache = PlanRuntimeCache::new();
    let first = cache.get_effective(&plan, 1, &ACTIVE_A);
    let second = cache.get_effective(&plan, 1, &ACTIVE_A);
    assert_eq!(first.schedules, second.schedules);
}

#[test]
fn cache_invalidates_on_revision_change() {
    let plan = ChannelPlan::default();
    let mut cache = PlanRuntimeCache::new();
    let rev1 = cache.get_effective(&plan, 1, &ACTIVE_A);
    let rev2 = cache.get_effective(&plan, 2, &ACTIVE_A);
    assert_eq!(rev1.schedules, rev2.schedules);
    assert_schedules_match_cache_and_direct(&mut cache, &plan, 2, &ACTIVE_A);
}

#[test]
fn cache_invalidates_on_active_sig_change() {
    let plan = ChannelPlan::default();
    let mut cache = PlanRuntimeCache::new();
    let a = cache.get_effective(&plan, 1, &ACTIVE_A);
    let b = cache.get_effective(&plan, 1, &ACTIVE_B);
    assert_ne!(hash_u32_list(&ACTIVE_A), hash_u32_list(&ACTIVE_B));
    assert_ne!(a.schedules.get(&31), b.schedules.get(&31));
    assert_schedules_match_cache_and_direct(&mut cache, &plan, 1, &ACTIVE_B);
}

#[test]
fn rebalance_due_matches_policy_wrapper() {
    let mut cache = PlanRuntimeCache::new();
    cache.record_push(3, 10);

    assert!(!rebalance_due(
        &cache,
        3,
        10,
        REBALANCE_MIN_INTERVAL_MS,
        REBALANCE_MIN_INTERVAL_MS,
    ));

    assert!(rebalance_due(&cache, 4, 10, 0, REBALANCE_MIN_INTERVAL_MS));

    assert!(!rebalance_due(
        &cache,
        3,
        11,
        REBALANCE_MIN_INTERVAL_MS - 1,
        REBALANCE_MIN_INTERVAL_MS,
    ));
    assert!(rebalance_due(
        &cache,
        3,
        11,
        REBALANCE_MIN_INTERVAL_MS,
        REBALANCE_MIN_INTERVAL_MS,
    ));
}

#[test]
fn auto_balance_three_sniffers_via_cache() {
    let plan = ChannelPlan::default();
    let mut cache = PlanRuntimeCache::new();
    let eff = cache.get_effective(&plan, 1, &ACTIVE_A);
    assert!(eff.unassigned.is_empty());
    assert_eq!(eff.schedules.len(), 3);
    for sched in eff.schedules.values() {
        assert!(!sched.pinned);
        assert!(sched.band.is_none());
    }
}
