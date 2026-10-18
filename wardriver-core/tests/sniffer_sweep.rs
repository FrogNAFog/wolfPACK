//! Host tests for sniffer sweep transforms and outbound grouping.

use std::collections::HashSet;

use wardriver_core::sniffer_sweep::{
    ble_lane_charge_bytes, filter_probe_groups_by_alerted_srcs, group_probe_rows, group_wifi_rows,
    had_wifi_bulk, local_probe_bursts_to_wire, merge_probe_bursts_by_src, merge_wifi_sweep_groups,
    oldest_probe_batch, oldest_wifi_batch, probe_batch_keys, probe_row_charge_bytes,
    throttle_wifi_entries_with_extras, wifi_batch_keys, wifi_row_charge_bytes, OutboundWifiRow,
    ProbeReqBurstInput,
};
use wardriver_proto::{ApEntryV1, AuthMini, ProbeReqBurstV1, ScanEntryExtras};

fn ap(bssid: u8, obs_mono_ms: u32, rssi: i8) -> ApEntryV1 {
    ApEntryV1 {
        bssid: [bssid; 6],
        channel: 6,
        rssi,
        auth: AuthMini::Open,
        ssid_len: 0,
        ssid: [0u8; 32],
        obs_mono_ms,
    }
}

fn probe(src: u8, channel: u8, first: u32, last: u32, wild: u8) -> ProbeReqBurstV1 {
    ProbeReqBurstV1 {
        src: [src; 6],
        wildcard_count: wild,
        channel,
        rssi_min: -70,
        rssi_max: -50,
        rssi_avg_q8: -60 << 8,
        first_seen_ms32: first,
        last_seen_ms32: last,
        primary_sig_matched: 0,
    }
}

#[test]
fn merge_probe_bursts_by_src_merges_same_mac_across_channels() {
    let src = [0xAA; 6];
    let g1 = vec![(100, 1_000, vec![probe(0xAA, 1, 10, 20, 2)])];
    let g2 = vec![(200, 2_000, vec![probe(0xAA, 6, 30, 40, 3)])];
    let mut groups = g1;
    groups.extend(g2);
    let merged = merge_probe_bursts_by_src(groups);
    assert_eq!(merged.len(), 1);
    let (scan_end_ms, scan_end_unix_ms, bursts) = &merged[0];
    assert_eq!(*scan_end_ms, 200);
    assert_eq!(*scan_end_unix_ms, 2_000);
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0].src, src);
    assert_eq!(bursts[0].wildcard_count, 5);
    assert_eq!(bursts[0].first_seen_ms32, 10);
    assert_eq!(bursts[0].last_seen_ms32, 40);
    assert_eq!(bursts[0].channel, 6);
}

#[test]
fn filter_probe_groups_keeps_only_alerted_macs() {
    let alerted: HashSet<[u8; 6]> = HashSet::from([[0x01; 6]]);
    let groups = vec![(
        100,
        1_000,
        vec![probe(0x01, 1, 0, 1, 1), probe(0x02, 1, 0, 1, 1)],
    )];
    let filtered = filter_probe_groups_by_alerted_srcs(groups, &alerted);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].2.len(), 1);
    assert_eq!(filtered[0].2[0].src, [0x01; 6]);
}

#[test]
fn throttle_wifi_entries_respects_wrapping_interval() {
    let entries = vec![
        ap(1, 1_000, -50),
        ap(1, 1_200, -50),
        ap(1, 1_600, -50),
        ap(2, 1_000, -50),
    ];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].obs_mono_ms, 1_200);
    assert_eq!(out[1].obs_mono_ms, 1_600);
    assert_eq!(out[2].bssid, [2; 6]);
}

#[test]
fn merge_wifi_sweep_groups_picks_max_scan_end_anchor() {
    let groups = vec![
        (100, 1_000, vec![ap(1, 10, -50)]),
        (300, 3_000, vec![ap(2, 20, -50)]),
        (200, 2_000, vec![ap(3, 30, -50)]),
    ];
    let merged = merge_wifi_sweep_groups(groups).expect("non-empty");
    assert_eq!(merged.0, 300);
    assert_eq!(merged.1, 3_000);
    assert_eq!(merged.2.len(), 3);
}

#[test]
fn group_wifi_rows_preserves_batch_order() {
    let rows = vec![
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(1, 10, -50),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 200,
            scan_end_unix_ms: 2_000,
            entry: ap(2, 20, -50),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(3, 30, -50),
            extras: Default::default(),
        },
    ];
    let grouped = group_wifi_rows(&rows);
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].0, 100);
    assert_eq!(grouped[0].2.len(), 2);
    assert_eq!(grouped[1].0, 200);
    assert_eq!(grouped[1].2.len(), 1);
}

#[test]
fn group_probe_rows_preserves_batch_order() {
    let rows = vec![
        (100, 1_000, probe(1, 1, 0, 1, 1)),
        (200, 2_000, probe(2, 6, 0, 1, 1)),
        (100, 1_000, probe(3, 1, 0, 1, 1)),
    ];
    let grouped = group_probe_rows(&rows);
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].2.len(), 2);
    assert_eq!(grouped[1].2.len(), 1);
}

#[test]
fn throttle_wifi_entries_keeps_strongest_within_window() {
    let entries = vec![
        ap(1, 1_000, -80),
        ap(1, 1_200, -60),
        ap(1, 1_600, -70),
        ap(2, 1_000, -55),
    ];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].rssi, -60);
    assert_eq!(out[0].obs_mono_ms, 1_200);
    assert_eq!(out[1].obs_mono_ms, 1_600);
    assert_eq!(out[1].rssi, -70);
    assert_eq!(out[2].bssid, [2; 6]);
}

#[test]
fn throttle_wifi_keeps_winning_rssi_extras() {
    let weak = ScanEntryExtras {
        rcoi_ie_payload: vec![0x01],
        ble_cod: None,
    };
    let strong = ScanEntryExtras {
        rcoi_ie_payload: vec![0x02, 0x03],
        ble_cod: None,
    };
    let entries = vec![ap(1, 1_000, -80), ap(1, 1_200, -60)];
    let extras = vec![weak, strong.clone()];
    let (out, out_ex) = throttle_wifi_entries_with_extras(entries, extras, 500);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rssi, -60);
    assert_eq!(out_ex[0], strong);
}

fn input_probe(src: u8, channel: u8, wild: u16) -> ProbeReqBurstInput {
    ProbeReqBurstInput {
        src: [src; 6],
        wildcard_count: wild,
        channel,
        first_seen_ms32: 100,
        last_seen_ms32: 200,
        rssi_min: -70,
        rssi_max: -50,
        rssi_avg_q8: -60 << 8,
        primary_sig_matched: false,
    }
}

// -- ble_lane_charge_bytes --

#[test]
fn ble_lane_charge_bytes_one_entry() {
    let bytes = ble_lane_charge_bytes(1);
    assert!(bytes > 0);
    assert_eq!(bytes, 4 + 8 + 1 * 36); // anchor + 1 * BLE_ENTRY_SIZE(36)
}

#[test]
fn ble_lane_charge_bytes_ten_entries() {
    let bytes = ble_lane_charge_bytes(10);
    assert_eq!(bytes, 4 + 8 + 10 * 36); // anchor + 10 * BLE_ENTRY_SIZE(36)
}

#[test]
fn ble_lane_charge_bytes_zero_entries() {
    assert_eq!(ble_lane_charge_bytes(0), 0);
}

// -- group_wifi_rows --

#[test]
fn group_wifi_rows_empty() {
    let rows: Vec<OutboundWifiRow> = vec![];
    let g = group_wifi_rows(&rows);
    assert!(g.is_empty());
}

// -- group_probe_rows --

#[test]
fn group_probe_rows_empty() {
    let rows: Vec<(u32, u64, ProbeReqBurstV1)> = vec![];
    let g = group_probe_rows(&rows);
    assert!(g.is_empty());
}

#[test]
fn group_probe_rows_groups_by_timestamp() {
    let rows = vec![
        (100, 1_000, probe(1, 1, 0, 1, 1)),
        (100, 1_000, probe(2, 1, 0, 1, 1)),
        (200, 2_000, probe(3, 1, 0, 1, 1)),
    ];
    let g = group_probe_rows(&rows);
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].0, 100);
    assert_eq!(g[0].2.len(), 2);
    assert_eq!(g[1].0, 200);
    assert_eq!(g[1].2.len(), 1);
}

// -- filter_probe_groups_by_alerted_srcs --

#[test]
fn filter_probe_groups_by_alerted_srcs_filters_nonalerted() {
    let alerted: HashSet<[u8; 6]> = HashSet::from([[0x01; 6]]);
    let groups = vec![(
        100,
        1_000,
        vec![probe(0x01, 1, 0, 1, 1), probe(0x02, 1, 0, 1, 1)],
    )];
    let filtered = filter_probe_groups_by_alerted_srcs(groups, &alerted);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].2.len(), 1);
    assert_eq!(filtered[0].2[0].src, [0x01; 6]);
}

#[test]
fn filter_probe_groups_by_alerted_srcs_all_filtered_removes_group() {
    let alerted: HashSet<[u8; 6]> = HashSet::from([[0x01; 6]]);
    let groups = vec![(
        100,
        1_000,
        vec![probe(0x02, 1, 0, 1, 1), probe(0x03, 1, 0, 1, 1)],
    )];
    let filtered = filter_probe_groups_by_alerted_srcs(groups, &alerted);
    assert!(filtered.is_empty());
}

// -- group_wifi_rows groups_by_timestamp --

#[test]
fn group_wifi_rows_groups_by_timestamp() {
    let rows = vec![
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(1, 10, -50),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(2, 20, -60),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 200,
            scan_end_unix_ms: 2_000,
            entry: ap(3, 30, -70),
            extras: Default::default(),
        },
    ];
    let g = group_wifi_rows(&rows);
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].2.len(), 2);
    assert_eq!(g[1].2.len(), 1);
}

// -- had_wifi_bulk --

#[test]
fn had_wifi_bulk_empty_groups_no_probes() {
    let groups: Vec<(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)> = vec![];
    let probes: Vec<(u32, u64, Vec<ProbeReqBurstV1>)> = vec![];
    assert!(!had_wifi_bulk(&groups, &probes));
}

#[test]
fn had_wifi_bulk_empty_groups_with_probes() {
    let groups: Vec<(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)> = vec![];
    let probes = vec![(100, 1_000, vec![probe(1, 1, 0, 1, 1)])];
    assert!(had_wifi_bulk(&groups, &probes));
}

#[test]
fn had_wifi_bulk_nonempty_groups() {
    let groups = vec![(100, 1_000, vec![ap(1, 10, -50)], vec![])];
    let probes: Vec<(u32, u64, Vec<ProbeReqBurstV1>)> = vec![];
    assert!(had_wifi_bulk(&groups, &probes));
}

// -- local_probe_bursts_to_wire --

#[test]
fn local_probe_bursts_to_wire_caps_at_32() {
    let bursts: Vec<ProbeReqBurstInput> = (0..35).map(|i| input_probe(i as u8, 1, 1)).collect();
    let wire = local_probe_bursts_to_wire(bursts);
    assert_eq!(wire.len(), 32);
}

#[test]
fn local_probe_bursts_to_wire_caps_wildcard_count_to_u8_max() {
    let bursts = vec![ProbeReqBurstInput {
        src: [1; 6],
        wildcard_count: 300,
        channel: 1,
        first_seen_ms32: 100,
        last_seen_ms32: 200,
        rssi_min: -70,
        rssi_max: -50,
        rssi_avg_q8: -60 << 8,
        primary_sig_matched: false,
    }];
    let wire = local_probe_bursts_to_wire(bursts);
    assert_eq!(wire[0].wildcard_count, 255);
}

#[test]
fn local_probe_bursts_to_wire_skips_zero_wildcard() {
    let bursts = vec![input_probe(1, 1, 0), input_probe(2, 1, 1)];
    let wire = local_probe_bursts_to_wire(bursts);
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0].src, [2; 6]);
}

// -- merge_probe_burst (inverted RSSI) --

#[test]
fn merge_probe_burst_both_inverted_rssi() {
    let g1 = vec![(
        100,
        1_000,
        vec![ProbeReqBurstV1 {
            src: [0xAA; 6],
            wildcard_count: 2,
            channel: 1,
            rssi_min: -30,
            rssi_max: -20,
            rssi_avg_q8: -25 << 8,
            first_seen_ms32: 10,
            last_seen_ms32: 20,
            primary_sig_matched: 0,
        }],
    )];
    let g2 = vec![(
        200,
        2_000,
        vec![ProbeReqBurstV1 {
            src: [0xAA; 6],
            wildcard_count: 3,
            channel: 6,
            rssi_min: -40,
            rssi_max: -30,
            rssi_avg_q8: -35 << 8,
            first_seen_ms32: 30,
            last_seen_ms32: 40,
            primary_sig_matched: 0,
        }],
    )];
    let merged = merge_probe_bursts_by_src([g1, g2].concat());
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].2[0].rssi_min, -40);
    assert_eq!(merged[0].2[0].rssi_max, -20);
}

#[test]
fn merge_probe_burst_extreme_rssi_values() {
    let g1 = vec![(
        100,
        1_000,
        vec![ProbeReqBurstV1 {
            src: [0xBB; 6],
            wildcard_count: 1,
            channel: 1,
            rssi_min: i8::MIN,
            rssi_max: i8::MAX,
            rssi_avg_q8: 0,
            first_seen_ms32: 10,
            last_seen_ms32: 20,
            primary_sig_matched: 0,
        }],
    )];
    let g2 = vec![(
        200,
        2_000,
        vec![ProbeReqBurstV1 {
            src: [0xBB; 6],
            wildcard_count: 1,
            channel: 6,
            rssi_min: i8::MIN,
            rssi_max: i8::MAX,
            rssi_avg_q8: 0,
            first_seen_ms32: 30,
            last_seen_ms32: 40,
            primary_sig_matched: 0,
        }],
    )];
    let merged = merge_probe_bursts_by_src([g1, g2].concat());
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].2[0].rssi_min, i8::MIN);
    assert_eq!(merged[0].2[0].rssi_max, i8::MAX);
}

#[test]
fn merge_probe_burst_inverted_rssi_min_max() {
    let g1 = vec![(
        100,
        1_000,
        vec![ProbeReqBurstV1 {
            src: [0xCC; 6],
            wildcard_count: 1,
            channel: 1,
            rssi_min: -50,
            rssi_max: -30,
            rssi_avg_q8: -40 << 8,
            first_seen_ms32: 10,
            last_seen_ms32: 20,
            primary_sig_matched: 0,
        }],
    )];
    let g2 = vec![(
        200,
        2_000,
        vec![ProbeReqBurstV1 {
            src: [0xCC; 6],
            wildcard_count: 1,
            channel: 6,
            rssi_min: -60,
            rssi_max: -20,
            rssi_avg_q8: -40 << 8,
            first_seen_ms32: 30,
            last_seen_ms32: 40,
            primary_sig_matched: 0,
        }],
    )];
    let merged = merge_probe_bursts_by_src([g1, g2].concat());
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].2[0].rssi_min, -60);
    assert_eq!(merged[0].2[0].rssi_max, -20);
}

#[test]
fn merge_probe_bursts_by_src_different_srcs_not_merged() {
    let groups = vec![
        (100, 1_000, vec![probe(0xAA, 1, 10, 20, 2)]),
        (100, 1_000, vec![probe(0xBB, 6, 30, 40, 3)]),
    ];
    let merged = merge_probe_bursts_by_src(groups);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].2.len(), 2);
}

#[test]
fn merge_probe_bursts_by_src_empty() {
    let groups: Vec<(u32, u64, Vec<ProbeReqBurstV1>)> = vec![];
    let merged = merge_probe_bursts_by_src(groups);
    assert!(merged.is_empty());
}

#[test]
fn merge_probe_bursts_by_src_same_src_merged() {
    let groups = vec![
        (100, 1_000, vec![probe(0xAA, 1, 10, 20, 2)]),
        (200, 2_000, vec![probe(0xAA, 6, 30, 40, 3)]),
    ];
    let merged = merge_probe_bursts_by_src(groups);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].2.len(), 1);
    assert_eq!(merged[0].2[0].src, [0xAA; 6]);
    assert_eq!(merged[0].2[0].wildcard_count, 5);
}

// -- merge_wifi_sweep_groups --

#[test]
fn merge_wifi_sweep_groups_empty() {
    let groups: Vec<(u32, u64, Vec<ApEntryV1>)> = vec![];
    assert!(merge_wifi_sweep_groups(groups).is_none());
}

#[test]
fn merge_wifi_sweep_groups_empty_inner_vectors() {
    let groups = vec![(100, 1_000, vec![]), (200, 2_000, vec![])];
    let merged = merge_wifi_sweep_groups(groups).expect("should have result");
    assert!(merged.2.is_empty());
    assert_eq!(merged.0, 200);
}

#[test]
fn merge_wifi_sweep_groups_multi_channels() {
    let groups = vec![
        (100, 1_000, vec![ap(1, 10, -50)]),
        (200, 2_000, vec![ap(2, 20, -60)]),
        (300, 3_000, vec![ap(3, 30, -70)]),
    ];
    let merged = merge_wifi_sweep_groups(groups).expect("non-empty");
    assert_eq!(merged.0, 300);
    assert_eq!(merged.1, 3_000);
    assert_eq!(merged.2.len(), 3);
}

#[test]
fn merge_wifi_sweep_groups_single() {
    let groups = vec![(100, 1_000, vec![ap(1, 10, -50)])];
    let merged = merge_wifi_sweep_groups(groups).expect("non-empty");
    assert_eq!(merged.0, 100);
    assert_eq!(merged.1, 1_000);
    assert_eq!(merged.2.len(), 1);
}

// -- oldest_probe_batch --

#[test]
fn oldest_probe_batch_empty() {
    let rows: Vec<(u32, u64, ProbeReqBurstV1)> = vec![];
    assert!(oldest_probe_batch(&rows).is_none());
}

#[test]
fn oldest_probe_batch_returns_min() {
    let rows = vec![
        (300, 3_000, probe(1, 1, 0, 1, 1)),
        (100, 1_000, probe(2, 1, 0, 1, 1)),
        (200, 2_000, probe(3, 1, 0, 1, 1)),
    ];
    assert_eq!(oldest_probe_batch(&rows), Some((100, 1_000)));
}

// -- oldest_wifi_batch --

#[test]
fn oldest_wifi_batch_empty() {
    let rows: Vec<OutboundWifiRow> = vec![];
    assert!(oldest_wifi_batch(&rows).is_none());
}

#[test]
fn oldest_wifi_batch_returns_min() {
    let rows = vec![
        OutboundWifiRow {
            scan_end_ms: 300,
            scan_end_unix_ms: 3_000,
            entry: ap(1, 10, -50),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(2, 20, -60),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 200,
            scan_end_unix_ms: 2_000,
            entry: ap(3, 30, -70),
            extras: Default::default(),
        },
    ];
    assert_eq!(oldest_wifi_batch(&rows), Some((100, 1_000)));
}

// -- probe_batch_keys --

#[test]
fn probe_batch_keys_deduplicates() {
    let rows = vec![
        (100, 1_000, probe(1, 1, 0, 1, 1)),
        (100, 1_000, probe(2, 1, 0, 1, 1)),
        (200, 2_000, probe(3, 1, 0, 1, 1)),
    ];
    let keys = probe_batch_keys(&rows);
    assert_eq!(keys.len(), 2);
}

// -- probe_row_charge_bytes --

#[test]
fn probe_row_charge_bytes_constant() {
    assert!(probe_row_charge_bytes() > 0);
}

// -- throttle --

#[test]
fn throttle_empty_input() {
    let entries: Vec<ApEntryV1> = vec![];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert!(out.is_empty());
}

#[test]
fn throttle_min_interval_default() {
    let entries = vec![ap(1, 1_000, -50), ap(1, 1_300, -60)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 1);
}

#[test]
fn throttle_multiple_bssids() {
    let entries = vec![ap(1, 1_000, -50), ap(2, 1_200, -60)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 2);
}

#[test]
fn throttle_rssi_tie_keeps_latest() {
    let entries = vec![ap(1, 1_000, -50), ap(1, 1_200, -50)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].obs_mono_ms, 1_200);
}

#[test]
fn throttle_same_bssid_at_exactly_min_interval() {
    let entries = vec![ap(1, 1_000, -50), ap(1, 1_500, -60)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 2);
}

#[test]
fn throttle_same_bssid_single_result() {
    let entries = vec![ap(1, 1_000, -50), ap(1, 1_200, -40), ap(1, 1_300, -60)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rssi, -40);
}

#[test]
fn throttle_same_bssid_within_window_300_then_800() {
    let entries = vec![ap(1, 1_000, -50), ap(1, 1_300, -40), ap(1, 1_800, -70)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].rssi, -40);
    assert_eq!(out[1].obs_mono_ms, 1_800);
}

#[test]
fn throttle_same_bssid_wrapping_past_u32_max() {
    let u32max = u32::MAX;
    let entries = vec![ap(1, u32max - 200, -50), ap(1, 300, -60)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    // 300.wrapping_sub(u32max-200) = 500, so exactly at boundary -> 2 results
    assert_eq!(out.len(), 2);
}

#[test]
fn throttle_wifi_entries_with_extras_preserves_extra_of_winner() {
    let weak_ex = ScanEntryExtras {
        rcoi_ie_payload: vec![0x01],
        ble_cod: None,
    };
    let strong_ex = ScanEntryExtras {
        rcoi_ie_payload: vec![0x02, 0x03],
        ble_cod: None,
    };
    let entries = vec![ap(1, 1_000, -80), ap(1, 1_200, -60)];
    let extras = vec![weak_ex, strong_ex];
    let (out, out_ex) = throttle_wifi_entries_with_extras(entries, extras, 500);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].rssi, -60);
    assert_eq!(out_ex[0].rcoi_ie_payload, vec![0x02, 0x03]);
}

// -- wifi_batch_keys --

#[test]
fn wifi_batch_keys_deduplicates() {
    let rows = vec![
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(1, 10, -50),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 100,
            scan_end_unix_ms: 1_000,
            entry: ap(2, 20, -60),
            extras: Default::default(),
        },
        OutboundWifiRow {
            scan_end_ms: 200,
            scan_end_unix_ms: 2_000,
            entry: ap(3, 30, -70),
            extras: Default::default(),
        },
    ];
    let keys = wifi_batch_keys(&rows);
    assert_eq!(keys.len(), 2);
}

#[test]
fn wifi_batch_keys_empty() {
    let rows: Vec<OutboundWifiRow> = vec![];
    assert!(wifi_batch_keys(&rows).is_empty());
}

// -- wifi_row_charge_bytes --

#[test]
fn wifi_row_charge_bytes_constant() {
    assert!(wifi_row_charge_bytes() > 0);
}
