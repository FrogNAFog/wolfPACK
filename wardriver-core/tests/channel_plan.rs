//! Host tests for channel plan (balance, edge cases, hop order).
use wardriver_core::channel_plan::{
    balance_channels_across_sniffers, default_channels_2_4, effective_from_plan, is_2_4_channel,
    is_5ghz_channel, master_scan_pool, normalize_wifi_channels, partition_channels_contiguously,
    wardrive_hop_channels, Band, BandAssignmentMode, ChannelPlan,
};
use wardriver_proto::CHANNELS_MAX;

fn bucket_sizes(buckets: &std::collections::BTreeMap<u32, Vec<u8>>) -> Vec<usize> {
    buckets.values().map(Vec::len).collect()
}

fn mixes_bands(bucket: &[u8]) -> bool {
    let has_24 = bucket.iter().any(|&ch| is_2_4_channel(ch));
    let has_5 = bucket.iter().any(|&ch| is_5ghz_channel(ch));
    has_24 && has_5
}

fn is_contiguous_subslice(pool: &[u8], bucket: &[u8]) -> bool {
    if bucket.is_empty() {
        return true;
    }
    let start = pool
        .iter()
        .position(|&ch| ch == bucket[0])
        .expect("bucket start in pool");
    pool[start..start + bucket.len()] == *bucket
}

#[test]
fn default_plan_enables_all_bands_and_auto_balance() {
    let plan = ChannelPlan::default();
    assert!(plan.include_2_4_ghz);
    assert!(plan.include_5_ghz);
    assert!(plan.include_dfs);
    assert_eq!(plan.band_mode, BandAssignmentMode::AutoBalance);
    assert_eq!(plan.channels_2_4, (1..=11).collect::<Vec<_>>());
    assert!(plan.sniffers_2_4.is_empty());
    assert!(plan.sniffers_5.is_empty());
    assert_eq!(master_scan_pool(&plan).len(), 39);
}

#[test]
fn auto_balance_three_sniffers_even_cross_band_buckets() {
    let active = [10u32, 20, 30];
    let plan = ChannelPlan::default();
    let pool = master_scan_pool(&plan);

    let balanced = balance_channels_across_sniffers(&plan, &active);
    assert!(balanced.unassigned.is_empty());

    let sizes = bucket_sizes(&balanced.buckets);
    assert_eq!(sizes.len(), 3);
    let min = *sizes.iter().min().unwrap();
    let max = *sizes.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "bucket sizes {sizes:?} differ by more than 1"
    );

    for bucket in balanced.buckets.values() {
        assert!(
            is_contiguous_subslice(&pool, bucket),
            "bucket {bucket:?} is not a contiguous pool slice"
        );
    }

    let mixed = balanced.buckets.values().any(|b| mixes_bands(b));
    assert!(mixed, "expected at least one bucket to mix 2.4 and 5 GHz");

    let eff = effective_from_plan(&plan, &active);
    for sched in eff.schedules.values() {
        assert!(!sched.pinned);
        assert!(sched.band.is_none());
    }
}

#[test]
fn band_mode_roundtrips_in_kv() {
    let mut plan = ChannelPlan {
        band_mode: BandAssignmentMode::Manual,
        ..ChannelPlan::default()
    };
    plan.sniffers_2_4 = vec![1, 2];

    let text = plan.encode_kv();
    let decoded = ChannelPlan::decode_kv(&text).expect("decode");
    assert_eq!(decoded.band_mode, BandAssignmentMode::Manual);
    assert_eq!(decoded.sniffers_2_4, vec![1, 2]);

    let auto = ChannelPlan::decode_kv("version=1\ninclude_2_4=1\n").expect("decode");
    assert_eq!(auto.band_mode, BandAssignmentMode::AutoBalance);
}

#[test]
fn manual_mode_leaves_unassigned_off_lists() {
    let active = [1u32, 2, 3];

    let plan = ChannelPlan {
        include_2_4_ghz: true,
        include_5_ghz: true,
        sniffers_2_4: vec![1],
        sniffers_5: vec![],
        band_mode: BandAssignmentMode::Manual,
        ..ChannelPlan::default()
    };

    assert_eq!(plan.sniffers_2_4, vec![1]);
    assert!(plan.sniffers_5.is_empty());
    assert!(!plan.sniffers_2_4.contains(&2));
    assert!(!plan.sniffers_2_4.contains(&3));

    let eff = effective_from_plan(&plan, &active);
    assert_eq!(eff.schedules.get(&1).unwrap().band, Some(Band::TwoFour));
    assert_eq!(eff.schedules.get(&2).unwrap().chan_count, 0);
}

#[test]
fn manual_mode_partitions_2_4_contiguously() {
    let active = [1u32, 2];
    let plan = ChannelPlan {
        include_2_4_ghz: true,
        include_5_ghz: false,
        include_dfs: false,
        sniffers_2_4: vec![1, 2],
        sniffers_5: vec![],
        band_mode: BandAssignmentMode::Manual,
        channels_2_4: (1..=11).collect(),
        ..ChannelPlan::default()
    };

    let eff = effective_from_plan(&plan, &active);
    let s1 = &eff.schedules.get(&1).unwrap().channels[..6];
    let s2 = &eff.schedules.get(&2).unwrap().channels[..5];
    assert_eq!(s1, &[1, 2, 3, 4, 5, 6]);
    assert_eq!(s2, &[7, 8, 9, 10, 11]);
}

#[test]
fn auto_balance_is_deterministic() {
    let active = [1u32, 2, 3, 4];
    let plan = ChannelPlan::default();

    let a = balance_channels_across_sniffers(&plan, &active);
    let b = balance_channels_across_sniffers(&plan, &active);
    assert_eq!(a, b);

    let eff_a = effective_from_plan(&plan, &active);
    let eff_b = effective_from_plan(&plan, &active);
    assert_eq!(eff_a.schedules, eff_b.schedules);
}

#[test]
fn single_sniffer_bucket_capped_at_channels_max() {
    let active = [99u32];
    let plan = ChannelPlan::default();
    let pool_len = master_scan_pool(&plan).len();
    assert!(
        pool_len > CHANNELS_MAX,
        "precondition: default pool exceeds cap"
    );

    let balanced = balance_channels_across_sniffers(&plan, &active);
    let bucket = balanced.buckets.get(&99).expect("sniffer bucket");
    assert_eq!(bucket.len(), CHANNELS_MAX);
    assert_eq!(balanced.unassigned.len(), pool_len - CHANNELS_MAX);

    let eff = effective_from_plan(&plan, &active);
    assert_eq!(
        eff.schedules.get(&99).unwrap().chan_count as usize,
        CHANNELS_MAX
    );
    assert_eq!(eff.unassigned.len(), pool_len - CHANNELS_MAX);
}

#[test]
fn partition_contiguously_balances_remainder() {
    let pool: Vec<u8> = (1..=11).collect();
    let slices = partition_channels_contiguously(&pool, 3);
    assert_eq!(slices[0], vec![1, 2, 3, 4]);
    assert_eq!(slices[1], vec![5, 6, 7, 8]);
    assert_eq!(slices[2], vec![9, 10, 11]);
}

#[test]
fn balance_with_zero_active_sniffers_returns_empty() {
    let plan = ChannelPlan::default();
    let buckets = balance_channels_across_sniffers(&plan, &[]);
    assert!(buckets.buckets.is_empty());
    // All channels should be unassigned
    let pool = plan.master_scan_pool();
    assert_eq!(buckets.unassigned.len(), pool.len());
}

#[test]
fn balance_with_one_sniffer_gets_channels() {
    let plan = ChannelPlan::default();
    let buckets = balance_channels_across_sniffers(&plan, &[1]);
    assert_eq!(buckets.buckets.len(), 1);
    let bucket = buckets.buckets.get(&1).unwrap();
    assert!(bucket.len() > 0, "sniffer should get channels");
    // Balance distributes channels by band; unassigned may be non-empty
    assert!(
        bucket.len() <= plan.master_scan_pool().len() + buckets.unassigned.len(),
        "bucket should not exceed pool"
    );
}

#[test]
fn normalize_removes_channel_zero() {
    let chs = vec![0, 1, 6, 11, 36];
    let norm = normalize_wifi_channels(chs);
    assert!(!norm.contains(&0), "channel 0 should be removed");
    assert!(norm.contains(&1));
    assert!(norm.contains(&36));
}

#[test]
fn normalize_removes_invalid_channels() {
    let chs = vec![1, 50, 99, 148, 197, 200];
    let norm = normalize_wifi_channels(chs);
    // 50, 99, 148, 197, 200 are not valid WiFi channels
    assert_eq!(norm, vec![1]);
}

#[test]
fn normalize_sorts_and_deduplicates() {
    let chs = vec![11, 6, 1, 6, 11, 1];
    let norm = normalize_wifi_channels(chs);
    assert_eq!(norm, vec![1, 6, 11]);
}

#[test]
fn decode_kv_missing_version_fails() {
    let text = "include_2_4=1\ninclude_5=0\ndwell_ms=200\n";
    let result = ChannelPlan::decode_kv(text);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("version"));
}

#[test]
fn decode_kv_unsupported_version_fails() {
    let text = "version=99\ninclude_2_4=1\n";
    let result = ChannelPlan::decode_kv(text);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("99"));
}

#[test]
fn decode_kv_truncated_line_is_ignored() {
    let text = "version=1\ninclude_2_4=1\nbad_line_no_equals\n";
    let plan = ChannelPlan::decode_kv(text).expect("parse");
    assert!(plan.include_2_4_ghz);
}

#[test]
fn decode_kv_with_invalid_dwell_clamps() {
    let text = "version=1\ninclude_2_4=1\ndwell_ms=50000\n";
    let plan = ChannelPlan::decode_kv(text).expect("parse");
    assert_eq!(plan.dwell_ms, 10_000); // clamped to MAX_DWELL_MS
}

#[test]
fn decode_kv_malformed_channel_list_skips_invalid() {
    let text = "version=1\ninclude_2_4=1\nchannels_2_4=1,abc,6,xyz,11\n";
    let plan = ChannelPlan::decode_kv(text).expect("parse");
    assert_eq!(plan.channels_2_4, vec![1, 6, 11]);
}

#[test]
fn decode_kv_roundtrips() {
    let mut plan = ChannelPlan::default();
    plan.dwell_ms = 300;
    plan.include_dfs = false;
    plan.sniffers_2_4 = vec![1, 2];
    plan.sniffers_5 = vec![3];

    let kv = plan.encode_kv();
    let decoded = ChannelPlan::decode_kv(&kv).expect("roundtrip");
    assert_eq!(decoded.dwell_ms, plan.dwell_ms);
    assert_eq!(decoded.include_2_4_ghz, plan.include_2_4_ghz);
    assert_eq!(decoded.include_5_ghz, plan.include_5_ghz);
    assert_eq!(decoded.include_dfs, plan.include_dfs);
    assert_eq!(decoded.channels_2_4, plan.channels_2_4);
    assert_eq!(decoded.channels_5, plan.channels_5);
    assert_eq!(decoded.sniffers_2_4, plan.sniffers_2_4);
    assert_eq!(decoded.sniffers_5, plan.sniffers_5);
    assert_eq!(decoded.band_mode, plan.band_mode);
}

#[test]
fn decode_kv_manual_band_mode() {
    let text = "version=1\nband_mode=manual\ninclude_2_4=1\ninclude_5=1\n";
    let plan = ChannelPlan::decode_kv(text).expect("parse");
    assert_eq!(plan.band_mode, BandAssignmentMode::Manual);
}

#[test]
fn decode_kv_default_band_mode() {
    let text = "version=1\ninclude_2_4=1\n";
    let plan = ChannelPlan::decode_kv(text).expect("parse");
    assert_eq!(plan.band_mode, BandAssignmentMode::AutoBalance);
}

#[test]
fn effective_with_no_channels_returns_empty_schedule() {
    let mut plan = ChannelPlan::default();
    plan.include_2_4_ghz = false;
    plan.include_5_ghz = false;
    plan.band_mode = BandAssignmentMode::AutoBalance;
    let eff = effective_from_plan(&plan, &[1]);
    let sched = eff.schedules.get(&1).expect("sniffer has schedule");
    assert_eq!(sched.chan_count, 0);
}

#[test]
fn dwell_at_min_boundary() {
    let mut plan = ChannelPlan::default();
    plan.dwell_ms = 50;
    plan.band_mode = BandAssignmentMode::AutoBalance;
    let eff = effective_from_plan(&plan, &[1]);
    let sched = eff.schedules.get(&1).expect("schedule");
    assert_eq!(sched.dwell_ms, 50);
}

#[test]
fn dwell_at_max_boundary() {
    let mut plan = ChannelPlan::default();
    plan.dwell_ms = 10_000;
    plan.band_mode = BandAssignmentMode::AutoBalance;
    let eff = effective_from_plan(&plan, &[1]);
    let sched = eff.schedules.get(&1).expect("schedule");
    assert_eq!(sched.dwell_ms, 10_000);
}

#[test]
fn normalize_does_not_accept_channel_15() {
    let chs = vec![1, 14, 15];
    let norm = normalize_wifi_channels(chs);
    assert!(!norm.contains(&15), "channel 15 is not valid");
    assert!(norm.contains(&14));
}

#[test]
fn empty_channel_list_in_kv_decodes_as_empty() {
    let text = "version=1\ninclude_2_4=1\nchannels_2_4=\n";
    let plan = ChannelPlan::decode_kv(text).expect("parse");
    assert!(plan.channels_2_4.is_empty());
}

#[test]
fn wardrive_hop_dense_contiguous_bucket_reverses() {
    assert_eq!(wardrive_hop_channels(&[1, 2, 3, 4]), vec![4, 3, 2, 1]);
}

#[test]
fn wardrive_hop_sparse_bucket_reverses_channel_numbers() {
    // Sparse 2.4 GHz buckets must visit highest channel first, not index N-1..0 on [1,6,11].
    assert_eq!(wardrive_hop_channels(&[1, 6, 11]), vec![11, 6, 1]);
}

#[test]
fn wardrive_hop_single_channel_unchanged() {
    // One-channel plans have no reordering work; reverse is a no-op.
    assert_eq!(wardrive_hop_channels(&[6]), vec![6]);
}

#[test]
fn wardrive_hop_empty_unchanged() {
    // Empty schedule yields empty hop list without panic.
    assert_eq!(wardrive_hop_channels(&[]), Vec::<u8>::new());
}
