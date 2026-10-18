//! Host tests for ESP-NOW send frag counting.

use wardriver_core::sniffer_frag::{
    count_ble_send_frags_with_extras, count_probe_send_frags, count_wifi_send_frags_with_extras,
    MAX_APS_PER_REASSEMBLY,
};
use wardriver_proto::{
    ApEntryV1, AuthMini, BleEntryV1, ProbeReqBurstV1, MAX_BLE_PER_FRAG, MAX_PROBE_REQ_PER_FRAG,
};

fn ap_entry(bssid: u8, obs_mono_ms: u32) -> ApEntryV1 {
    ApEntryV1 {
        bssid: [bssid; 6],
        channel: 6,
        rssi: -50,
        auth: AuthMini::Open,
        ssid_len: 0,
        ssid: [0u8; 32],
        obs_mono_ms,
    }
}

fn probe_burst(src: u8) -> ProbeReqBurstV1 {
    ProbeReqBurstV1 {
        src: [src; 6],
        wildcard_count: 3,
        channel: 6,
        rssi_min: -70,
        rssi_max: -50,
        rssi_avg_q8: -60 << 8,
        first_seen_ms32: 100,
        last_seen_ms32: 200,
        primary_sig_matched: 0,
    }
}

#[test]
fn ble_zero_entries_zero_frags() {
    assert_eq!(count_ble_send_frags_with_extras(&[], &[]), 0);
}

#[test]
fn ble_one_entry_one_frag() {
    let entries = vec![BleEntryV1 {
        addr: [1, 2, 3, 4, 5, 6],
        rssi: -55,
        addr_type: 0,
        company_id: 0,
        name: [0u8; 22],
        obs_mono_ms: 0,
    }];
    assert_eq!(count_ble_send_frags_with_extras(&entries, &[]), 1);
}

#[test]
fn ble_max_per_frag_boundary() {
    let entries: Vec<_> = (0..MAX_BLE_PER_FRAG)
        .map(|i| BleEntryV1 {
            addr: [i as u8; 6],
            rssi: -55,
            addr_type: 0,
            company_id: 0,
            name: [0u8; 22],
            obs_mono_ms: 0,
        })
        .collect();
    assert_eq!(count_ble_send_frags_with_extras(&entries, &[]), 1);
    let mut plus_one = entries.clone();
    plus_one.push(entries[0]);
    assert_eq!(count_ble_send_frags_with_extras(&plus_one, &[]), 2);
}

#[test]
fn ble_overflow_past_u8_max_frags_returns_zero() {
    let entries: Vec<_> = (0..(u8::MAX as usize + 1) * MAX_BLE_PER_FRAG)
        .map(|i| BleEntryV1 {
            addr: [(i % 256) as u8; 6],
            rssi: -55,
            addr_type: 0,
            company_id: 0,
            name: [0u8; 22],
            obs_mono_ms: 0,
        })
        .collect();
    assert_eq!(count_ble_send_frags_with_extras(&entries, &[]), 0);
}

#[test]
fn wifi_empty_batches_zero_frags() {
    assert_eq!(count_wifi_send_frags_with_extras(&[]), 0);
    assert_eq!(
        count_wifi_send_frags_with_extras(&[(100, 1_000, vec![], vec![])]),
        0
    );
}

#[test]
fn wifi_frag_count_matches_chunk_helper() {
    let aps: Vec<_> = (0..3).map(|i| ap_entry(i, i as u32 * 10)).collect();
    let expected = wardriver_proto::count_scan_frag_chunks_with_extras(&aps, &[]) as u32;
    assert_eq!(
        count_wifi_send_frags_with_extras(&[(500, 1_700_000_000_000, aps, vec![])]),
        expected
    );
}

#[test]
fn wifi_large_batch_splits_on_reassembly_cap() {
    let n = MAX_APS_PER_REASSEMBLY + 5;
    let aps: Vec<_> = (0..n)
        .map(|i| ap_entry((i % 256) as u8, i as u32))
        .collect();
    let slice0 = &aps[..MAX_APS_PER_REASSEMBLY];
    let slice1 = &aps[MAX_APS_PER_REASSEMBLY..];
    let expected = wardriver_proto::count_scan_frag_chunks_with_extras(slice0, &[]) as u32
        + wardriver_proto::count_scan_frag_chunks_with_extras(slice1, &[]) as u32;
    assert_eq!(
        count_wifi_send_frags_with_extras(&[(500, 1_700_000_000_000, aps, vec![])]),
        expected
    );
}

#[test]
fn probe_empty_groups_zero_frags() {
    assert_eq!(count_probe_send_frags(&[]), 0);
}

#[test]
fn probe_single_and_multi_frag_groups() {
    let one = vec![(100, 1_000, vec![probe_burst(1)])];
    assert_eq!(count_probe_send_frags(&one), 1);

    let bursts: Vec<_> = (0..MAX_PROBE_REQ_PER_FRAG + 1)
        .map(|i| probe_burst(i as u8))
        .collect();
    let multi = vec![(100, 1_000, bursts)];
    assert_eq!(count_probe_send_frags(&multi), 2);
}

#[test]
fn wifi_extras_increase_send_frag_count() {
    let aps: Vec<_> = (0..12).map(|i| ap_entry(i, i as u32)).collect();
    let fat = wardriver_proto::ScanEntryExtras {
        rcoi_ie_payload: vec![0xAA; 180],
        ble_cod: None,
    };
    let extras = vec![fat; aps.len()];
    let without =
        count_wifi_send_frags_with_extras(&[(500, 1_700_000_000_000, aps.clone(), vec![])]);
    let with = count_wifi_send_frags_with_extras(&[(500, 1_700_000_000_000, aps, extras)]);
    assert!(with > without);
}
