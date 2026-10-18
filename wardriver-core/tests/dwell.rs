//! Host tests for sweep / silence budgets in `wardriver_core::dwell`.

use wardriver_core::channel_plan::ChannelPlan;
use wardriver_core::dwell::DwellAndSchedule;

#[test]
fn wifi_sweep_wall_includes_hop_budget() {
    let sched = DwellAndSchedule {
        dwell_ms: 200,
        channel_count: 4,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    assert_eq!(sched.listen_wall_ms(), 800);
    // listen < max_off: 3_000 ms cap + 4 hops × 10 ms proto budget
    assert_eq!(sched.sweep_wall_ms(), 3_040);
}

#[test]
fn wifi_sweep_wall_small_bucket_uses_max_off_cap() {
    for (channel_count, hop_overhead_ms) in [(1, 10), (2, 20), (3, 30)] {
        let sched = DwellAndSchedule {
            dwell_ms: 200,
            channel_count,
            role_is_ble: false,
            set_channel_us_avg: 0,
        };
        assert!(
            sched.listen_wall_ms() < wardriver_proto::SNIFFER_MAX_OFF_CONTROL_MS,
            "precondition: {channel_count} ch"
        );
        assert_eq!(
            sched.sweep_wall_ms(),
            wardriver_proto::SNIFFER_MAX_OFF_CONTROL_MS + hop_overhead_ms,
            "{channel_count} ch"
        );
    }
}

#[test]
fn wifi_sweep_wall_large_bucket_uses_rotation_listen() {
    let sched = DwellAndSchedule {
        dwell_ms: 500,
        channel_count: 25,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    assert_eq!(sched.listen_wall_ms(), 12_500);
    // listen ≥ max_off: rotation listen + 25 hops × 10 ms
    assert_eq!(sched.sweep_wall_ms(), 12_750);
}

#[test]
fn dwell_ms_from_plan_clamps() {
    let mut plan = ChannelPlan::default();
    plan.dwell_ms = 20_000;
    assert_eq!(DwellAndSchedule::dwell_ms_from_plan(&plan), 10_000);
    plan.dwell_ms = 10;
    assert_eq!(DwellAndSchedule::dwell_ms_from_plan(&plan), 50);
}

#[test]
fn ble_sweep_wall_uses_fixed_scan_window() {
    let sched = DwellAndSchedule {
        dwell_ms: 500,
        channel_count: 25,
        role_is_ble: true,
        set_channel_us_avg: 0,
    };
    assert_eq!(
        sched.sweep_wall_ms(),
        wardriver_proto::BLE_SCAN_WINDOW_MS + 100
    );
}

#[test]
fn dwell_ms_zero_clamps_to_minimum() {
    let sched = DwellAndSchedule {
        dwell_ms: DwellAndSchedule::clamp_dwell_ms(0),
        channel_count: 4,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    assert_eq!(sched.dwell_ms, wardriver_core::channel_plan::MIN_DWELL_MS);
}

#[test]
fn dwell_ms_u16_max_clamps_to_maximum() {
    let sched = DwellAndSchedule {
        dwell_ms: DwellAndSchedule::clamp_dwell_ms(u16::MAX),
        channel_count: 4,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    assert_eq!(sched.dwell_ms, wardriver_core::channel_plan::MAX_DWELL_MS);
}

#[test]
fn channel_count_zero_uses_one_for_wall_time() {
    let sched = DwellAndSchedule {
        dwell_ms: 200,
        channel_count: 0,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    assert_eq!(sched.channel_count, 0);
    // listen_wall_ms uses max(1) so it never divides by zero
    assert_eq!(sched.listen_wall_ms(), 200);
}

#[test]
fn sweep_wall_with_extreme_set_channel_us_avg() {
    let sched = DwellAndSchedule {
        dwell_ms: 200,
        channel_count: 4,
        role_is_ble: false,
        set_channel_us_avg: 100_000,
    };
    // hop overhead should use measured value when higher than proto budget
    let wall = sched.sweep_wall_ms();
    assert!(wall > 400, "hop overhead should dominate: {}", wall);
}

#[test]
fn sweep_wall_ms_no_overflow_with_max_dwell_and_channels() {
    let sched = DwellAndSchedule {
        dwell_ms: 10_000,
        channel_count: 64,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    // 10_000 * 64 = 640_000 ms — well within u64
    let wall = sched.sweep_wall_ms();
    assert!(wall > 600_000, "should be ~640k: {}", wall);
    assert!(wall < u64::MAX / 2, "should not overflow: {}", wall);
}

#[test]
fn control_silence_timeout_floors_at_minimum() {
    let sched = DwellAndSchedule {
        dwell_ms: 50,
        channel_count: 1,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    let timeout = sched.control_silence_timeout_ms(0);
    assert!(
        timeout >= wardriver_core::dwell::CONTROL_SILENCE_TIMEOUT_FLOOR_MS,
        "floor: {}",
        timeout
    );
}

#[test]
fn control_silence_timeout_adds_extra_allowance() {
    let sched = DwellAndSchedule {
        dwell_ms: 200,
        channel_count: 4,
        role_is_ble: false,
        set_channel_us_avg: 0,
    };
    let base = sched.control_silence_timeout_ms(0);
    let with_extra = sched.control_silence_timeout_ms(5_000);
    assert_eq!(with_extra, base + 5_000);
}

#[test]
fn ble_role_ignores_channel_count() {
    let sched1 = DwellAndSchedule {
        dwell_ms: 500,
        channel_count: 1,
        role_is_ble: true,
        set_channel_us_avg: 0,
    };
    let sched2 = DwellAndSchedule {
        dwell_ms: 500,
        channel_count: 20,
        role_is_ble: true,
        set_channel_us_avg: 0,
    };
    assert_eq!(
        sched1.sweep_wall_ms(),
        sched2.sweep_wall_ms(),
        "BLE should ignore channel count"
    );
}

#[test]
fn clamp_dwell_at_exact_boundaries() {
    assert_eq!(
        DwellAndSchedule::clamp_dwell_ms(50),
        wardriver_core::channel_plan::MIN_DWELL_MS
    );
    assert_eq!(
        DwellAndSchedule::clamp_dwell_ms(10_000),
        wardriver_core::channel_plan::MAX_DWELL_MS
    );
}

#[test]
fn plan_dwell_below_min_clamps() {
    let mut plan = ChannelPlan::default();
    plan.dwell_ms = 10;
    assert_eq!(DwellAndSchedule::dwell_ms_from_plan(&plan), 50);
}

#[test]
fn plan_dwell_above_max_clamps() {
    let mut plan = ChannelPlan::default();
    plan.dwell_ms = 50_000;
    assert_eq!(DwellAndSchedule::dwell_ms_from_plan(&plan), 10_000);
}
