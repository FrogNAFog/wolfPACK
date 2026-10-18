//! Golden JSON for `/api/stats` sniffer slot snapshots (field-name stability).

use wardriver_stats::SnifferSlotSnapshot;

#[test]
fn golden_sniffer_slot_snapshot_json() {
    let snap = SnifferSlotSnapshot {
        sniffer_id: 3,
        mac: "02:11:22:33:44:55".to_string(),
        last_seen_ms: 120_000,
        frames: 5,
        wifi_frag_batches: 3,
        ble_frag_batches: 2,
        ap_entries: 40,
        ble_entries: 8,
        state: 2,
        ack_cmd_seq: 99,
        role: 0,
        ble_capable: 1,
        five_ghz_capable: 1,
        dfs_capable: 1,
        reasm_dropped: 0,
        wifi_reasm_dropped: 0,
        ble_reasm_dropped: 0,
        rows_enqueued: 12,
        wifi_rows_enqueued: 10,
        ble_rows_enqueued: 2,
        mem_heap_free: 90_000,
        mem_heap_total: 200_000,
        mem_heap_avail_pct: Some(45),
        cpu_load_pct: 30,
        assigned_band: 1,
        pinned_channel: 0,
        assigned_channels: vec![36, 40, 44],
        bulk_lost_cycles: 0,
        wifi_bulk_lost_cycles: 0,
        ble_bulk_lost_cycles: 0,
        sniffer_boot_id: 0x1234_5678,
        reboot_count: 1,
        fleet_slot: 2,
        adoption_state: 1,
        last_dump_visit_ms: 1_000,
        last_dump_bulk_ms: 1_100,
        last_dump_post_ms: 1_200,
        last_dump_done_ms: 1_300,
        last_dump_queue_ms: 1_400,
        last_dump_bulk_xfer_ms: 1_500,
        time_sync_rtt_ms: 12,
        time_sync_one_way_ms: 6,
        last_rtt_time_set_ms: 2_000,
    };
    let json = serde_json::to_string(&snap).expect("serialize snapshot");
    let golden = include_str!("fixtures/snapshot_minimal.json").trim();
    // Compare parsed values so pretty-printed fixtures stay valid.
    let actual: serde_json::Value = serde_json::from_str(&json).expect("parse actual JSON");
    let expected: serde_json::Value = serde_json::from_str(golden).expect("parse golden JSON");
    assert_eq!(
        actual, expected,
        "SnifferSlotSnapshot JSON shape changed; refresh fixture"
    );
}

/// SnifferSlotSnapshot with mem_heap_avail_pct = None — serialization omits it.
#[test]
fn snapshot_mem_heap_avail_pct_none_omitted() {
    let snapshot = SnifferSlotSnapshot {
        sniffer_id: 1,
        mac: "aa:bb:cc:dd:ee:01".to_string(),
        last_seen_ms: 1_700_000_000_000,
        frames: 100,
        wifi_frag_batches: 50,
        ble_frag_batches: 50,
        ap_entries: 200,
        ble_entries: 100,
        state: 1,
        ack_cmd_seq: 5,
        role: 1,
        ble_capable: 1,
        five_ghz_capable: 1,
        dfs_capable: 0,
        reasm_dropped: 0,
        wifi_reasm_dropped: 0,
        ble_reasm_dropped: 0,
        rows_enqueued: 300,
        wifi_rows_enqueued: 200,
        ble_rows_enqueued: 100,
        mem_heap_free: 100_000,
        mem_heap_total: 200_000,
        mem_heap_avail_pct: None,
        cpu_load_pct: 5,
        assigned_band: 1,
        pinned_channel: 0,
        assigned_channels: vec![1, 6, 11],
        bulk_lost_cycles: 0,
        wifi_bulk_lost_cycles: 0,
        ble_bulk_lost_cycles: 0,
        sniffer_boot_id: 42,
        reboot_count: 0,
        fleet_slot: 1,
        adoption_state: 1,
        last_dump_visit_ms: 1_700_000_000_000,
        last_dump_bulk_ms: 1_700_000_000_100,
        last_dump_post_ms: 1_700_000_000_200,
        last_dump_done_ms: 1_700_000_000_300,
        last_dump_queue_ms: 0,
        last_dump_bulk_xfer_ms: 0,
        time_sync_rtt_ms: 0,
        time_sync_one_way_ms: 0,
        last_rtt_time_set_ms: 0,
    };
    let json = serde_json::to_string(&snapshot).expect("serialize");
    // #[derive(Serialize)] serializes Option::None as `null`, not omitted.
    assert!(
        json.contains("mem_heap_avail_pct"),
        "field should be present"
    );
    assert!(json.contains("null"), "None should serialize as null");
}

/// SnifferSlotSnapshot with mem_heap_avail_pct = Some(75) — serialized explicitly.
#[test]
fn snapshot_mem_heap_avail_pct_some_included() {
    let snapshot = SnifferSlotSnapshot {
        sniffer_id: 2,
        mac: "aa:bb:cc:dd:ee:02".to_string(),
        last_seen_ms: 1_700_000_000_100,
        frames: 500,
        wifi_frag_batches: 250,
        ble_frag_batches: 250,
        ap_entries: 1000,
        ble_entries: 500,
        state: 2,
        ack_cmd_seq: 10,
        role: 2,
        ble_capable: 1,
        five_ghz_capable: 0,
        dfs_capable: 0,
        reasm_dropped: 3,
        wifi_reasm_dropped: 1,
        ble_reasm_dropped: 2,
        rows_enqueued: 1500,
        wifi_rows_enqueued: 1000,
        ble_rows_enqueued: 500,
        mem_heap_free: 50_000,
        mem_heap_total: 200_000,
        mem_heap_avail_pct: Some(75),
        cpu_load_pct: 42,
        assigned_band: 1,
        pinned_channel: 6,
        assigned_channels: vec![1, 6, 11],
        bulk_lost_cycles: 3,
        wifi_bulk_lost_cycles: 2,
        ble_bulk_lost_cycles: 1,
        sniffer_boot_id: 43,
        reboot_count: 1,
        fleet_slot: 2,
        adoption_state: 2,
        last_dump_visit_ms: 1_700_000_000_100,
        last_dump_bulk_ms: 1_700_000_000_200,
        last_dump_post_ms: 1_700_000_000_300,
        last_dump_done_ms: 1_700_000_000_400,
        last_dump_queue_ms: 1_700_000_000_050,
        last_dump_bulk_xfer_ms: 1_700_000_000_150,
        time_sync_rtt_ms: 12,
        time_sync_one_way_ms: 6,
        last_rtt_time_set_ms: 1_700_000_000_100,
    };
    let json = serde_json::to_string(&snapshot).expect("serialize");
    assert!(json.contains("mem_heap_avail_pct"));
    assert!(json.contains("75"));
}

/// SnifferSlotSnapshot with very large assigned_channels vector (32 channels).
#[test]
fn snapshot_large_channels_vector() {
    let channels: Vec<u8> = (1..=32).collect();
    let snapshot = SnifferSlotSnapshot {
        sniffer_id: 3,
        mac: "aa:bb:cc:dd:ee:03".to_string(),
        last_seen_ms: 1_700_000_000_200,
        frames: 1000,
        wifi_frag_batches: 500,
        ble_frag_batches: 500,
        ap_entries: 2000,
        ble_entries: 1000,
        state: 2,
        ack_cmd_seq: 20,
        role: 1,
        ble_capable: 0,
        five_ghz_capable: 1,
        dfs_capable: 1,
        reasm_dropped: 0,
        wifi_reasm_dropped: 0,
        ble_reasm_dropped: 0,
        rows_enqueued: 3000,
        wifi_rows_enqueued: 2000,
        ble_rows_enqueued: 1000,
        mem_heap_free: 80_000,
        mem_heap_total: 200_000,
        mem_heap_avail_pct: Some(40),
        cpu_load_pct: 15,
        assigned_band: 2,
        pinned_channel: 0,
        assigned_channels: channels,
        bulk_lost_cycles: 0,
        wifi_bulk_lost_cycles: 0,
        ble_bulk_lost_cycles: 0,
        sniffer_boot_id: 44,
        reboot_count: 0,
        fleet_slot: 3,
        adoption_state: 2,
        last_dump_visit_ms: 1_700_000_000_200,
        last_dump_bulk_ms: 1_700_000_000_300,
        last_dump_post_ms: 1_700_000_000_400,
        last_dump_done_ms: 1_700_000_000_500,
        last_dump_queue_ms: 0,
        last_dump_bulk_xfer_ms: 0,
        time_sync_rtt_ms: 0,
        time_sync_one_way_ms: 0,
        last_rtt_time_set_ms: 0,
    };
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse back");
    let arr = parsed["assigned_channels"].as_array().expect("is array");
    assert_eq!(arr.len(), 32);
}

/// SnifferSlotSnapshot with zero-value fields — they serialize explicitly (not omitted).
#[test]
fn snapshot_zero_value_fields_serialized() {
    let snapshot = SnifferSlotSnapshot {
        sniffer_id: 0,
        mac: "00:00:00:00:00:00".to_string(),
        last_seen_ms: 0,
        frames: 0,
        wifi_frag_batches: 0,
        ble_frag_batches: 0,
        ap_entries: 0,
        ble_entries: 0,
        state: 0,
        ack_cmd_seq: 0,
        role: 1,
        ble_capable: 0,
        five_ghz_capable: 0,
        dfs_capable: 0,
        reasm_dropped: 0,
        wifi_reasm_dropped: 0,
        ble_reasm_dropped: 0,
        rows_enqueued: 0,
        wifi_rows_enqueued: 0,
        ble_rows_enqueued: 0,
        mem_heap_free: 0,
        mem_heap_total: 0,
        mem_heap_avail_pct: None,
        cpu_load_pct: 0,
        assigned_band: 0,
        pinned_channel: 0,
        assigned_channels: vec![],
        bulk_lost_cycles: 0,
        wifi_bulk_lost_cycles: 0,
        ble_bulk_lost_cycles: 0,
        sniffer_boot_id: 0,
        reboot_count: 0,
        fleet_slot: 0,
        adoption_state: 0,
        last_dump_visit_ms: 0,
        last_dump_bulk_ms: 0,
        last_dump_post_ms: 0,
        last_dump_done_ms: 0,
        last_dump_queue_ms: 0,
        last_dump_bulk_xfer_ms: 0,
        time_sync_rtt_ms: 0,
        time_sync_one_way_ms: 0,
        last_rtt_time_set_ms: 0,
    };
    let json = serde_json::to_string(&snapshot).expect("serialize");
    assert!(json.contains(r#""mem_heap_free":0"#));
    assert!(json.contains(r#""cpu_load_pct":0"#));
    assert!(json.contains(r#""last_seen_ms":0"#));
    assert!(json.contains(r#""assigned_channels":[]"#));
}
