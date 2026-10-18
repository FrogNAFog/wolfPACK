//! Host tests for MacTable edge cases.

use wardriver_core::mac_table::{
    consider_wigle_ble, consider_wigle_wifi, haversine_m, update_running_avg_i8, LaneFlags,
    MacEntry, MacTable, WIGLE_MAX_ROWS_PER_MAC,
};

#[test]
fn mac_table_at_full_capacity_evicts_lru() {
    let mut table = MacTable::new(3, 2);

    let mac1 = [1u8, 0, 0, 0, 0, 1];
    let mac2 = [1u8, 0, 0, 0, 0, 2];
    let mac3 = [1u8, 0, 0, 0, 0, 3];
    let mac4 = [1u8, 0, 0, 0, 0, 4];

    let r1 = table.allow(&mac1, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(r1.decision.allow_row);
    let r2 = table.allow(&mac2, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(r2.decision.allow_row);
    let r3 = table.allow(&mac3, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(r3.decision.allow_row);

    let r4 = table.allow(&mac4, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(r4.decision.allow_row);
    assert_eq!(r4.row_evictions, 1);
    assert!(table.get(&mac1).is_none(), "mac1 should be evicted");
    assert!(table.get(&mac4).is_some());
}

#[test]
fn wigle_row_count_cap_blocks_repeat() {
    let mut table = MacTable::new(10, 5);
    let mac = [2u8, 0, 0, 0, 0, 1];

    table.allow(&mac, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    let entry = table.get(&mac).unwrap();
    assert_eq!(entry.wigle_row_count, 1);

    for i in 2..=WIGLE_MAX_ROWS_PER_MAC {
        let lat = 37.0 + (i as f64) * 0.001;
        table.allow(&mac, lat, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    }

    let entry = table.get(&mac).unwrap();
    assert_eq!(entry.wigle_row_count, WIGLE_MAX_ROWS_PER_MAC);

    let lat = 37.0 + ((WIGLE_MAX_ROWS_PER_MAC as u8 + 1) as f64) * 0.001;
    let result = table.allow(&mac, lat, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(!result.decision.allow_row);
}

#[test]
fn rapid_position_jump_haversine_does_not_overflow() {
    let dist = haversine_m(40.7128, -74.0060, 31.2304, 121.4737);
    assert!(dist.is_finite(), "distance should be finite: {}", dist);
    assert!(dist > 10_000_000.0, "NYC to Shanghai should be > 10,000 km");
}

#[test]
fn same_point_has_zero_distance() {
    let dist = haversine_m(37.0, -122.0, 37.0, -122.0);
    assert!(
        (dist - 0.0).abs() < 0.001,
        "same point should be ~0: {}",
        dist
    );
}

#[test]
fn haversine_at_antipodes() {
    let dist = haversine_m(0.0, 0.0, 0.0, 180.0);
    assert!(dist.is_finite());
    assert!(
        dist > 10_000_000.0,
        "antipodes should be ~20,000 km: {}",
        dist
    );
}

#[test]
fn haversine_at_poles() {
    let dist = haversine_m(90.0, 0.0, 89.0, 0.0);
    assert!(dist.is_finite(), "pole distance should be finite: {}", dist);
    assert!(dist > 0.0);
}

#[test]
fn mac_first_seen_on_ble_then_wifi() {
    let mut table = MacTable::new(10, 5);
    let mac = [3u8, 0, 0, 0, 0, 1];

    let r1 = table.allow(&mac, 37.0, -122.0, LaneFlags::WIGLE_BLE, 50.0, Some(-55));
    assert!(r1.decision.new_unique);
    assert!(r1.decision.allow_row);

    let r2 = table.allow(&mac, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(r2.decision.allow_row);

    let entry = table.get(&mac).unwrap();
    assert!(entry.lanes.has(LaneFlags::WIGLE_BLE));
    assert!(entry.lanes.has(LaneFlags::WIGLE_WIFI));
    assert_eq!(entry.wigle_row_count, 2);
}

#[test]
fn lpr_camera_wifi_after_exhausting_wigle_rows() {
    let mut table = MacTable::new(10, 5);
    let mac = [4u8, 0, 0, 0, 0, 1];

    for i in 0..WIGLE_MAX_ROWS_PER_MAC as usize {
        let lat = 37.0 + (i as f64) * 0.001;
        table.allow(&mac, lat, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    }

    let _row_count = table.get(&mac).unwrap().wigle_row_count;
    assert_eq!(_row_count, WIGLE_MAX_ROWS_PER_MAC);

    let r = table.allow(
        &mac,
        37.0 + 100.0 * 0.001,
        -122.0,
        LaneFlags::DETECTION_WIFI,
        50.0,
        None,
    );
    assert!(
        r.decision.allow_row,
        "flock lane should allow after wigle cap"
    );
}

#[test]
fn running_average_does_not_overflow() {
    let _avg = update_running_avg_i8(127, 10, -128);
}

#[test]
fn consider_wigle_with_nan_coords_rejected() {
    let decision = consider_wigle_wifi(None, f64::NAN, -122.0, 50.0, -60);
    assert!(!decision.allow_row);
}

#[test]
fn consider_wigle_with_infinite_lon_rejected() {
    let decision = consider_wigle_wifi(None, 37.0, f64::INFINITY, 50.0, -60);
    assert!(!decision.allow_row);
}

#[test]
fn consider_ble_with_nan_coords_rejected() {
    let decision = consider_wigle_ble(None, f64::NAN, -122.0, 50.0, -60);
    assert!(!decision.allow_row);
}

#[test]
fn remove_nonexistent_mac_returns_false() {
    let mut table = MacTable::new(10, 5);
    let mac = [99u8, 0, 0, 0, 0, 1];
    assert!(!table.remove(&mac));
}

#[test]
fn remove_existing_mac_decrements_geo_count() {
    let mut table = MacTable::new(10, 5);
    let mac = [50u8, 0, 0, 0, 0, 1];
    let r = table.allow(&mac, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert_eq!(table.geo_active_count(), 1);
    assert!(table.remove(&mac));
    assert_eq!(table.geo_active_count(), 0);
}

#[test]
fn geo_slot_exhaustion_releases_lru() {
    let mut table = MacTable::new(10, 2);

    let mac1 = [60u8, 0, 0, 0, 0, 1];
    let mac2 = [60u8, 0, 0, 0, 0, 2];
    let mac3 = [60u8, 0, 0, 0, 0, 3];

    table.allow(&mac1, 37.0, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    table.allow(&mac2, 37.1, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert_eq!(table.geo_active_count(), 2);

    let r = table.allow(&mac3, 37.2, -122.0, LaneFlags::WIGLE_WIFI, 50.0, Some(-60));
    assert!(r.decision.allow_row);
    assert_eq!(r.geo_releases, 1);
    assert_eq!(table.geo_active_count(), 2);

    let e1 = table.get(&mac1).unwrap();
    assert!(!e1.geo_active, "mac1 geo should be deactivated");
}

#[test]
fn shrink_to_cap_fraction() {
    let mut table = MacTable::new(4, 4);

    for i in 0..4 {
        let mac = [70u8, i as u8, 0, 0, 0, 1];
        table.allow(
            &mac,
            37.0 + i as f64 * 0.01,
            -122.0,
            LaneFlags::WIGLE_WIFI,
            50.0,
            Some(-60),
        );
    }

    assert_eq!(table.len(), 4);
    let evicted = table.shrink_to_cap_fraction(0.5);
    assert_eq!(table.len(), 2);
    assert_eq!(evicted, 2);
}

#[test]
fn update_running_avg_single_value() {
    let avg = update_running_avg_i8(0, 0, -60);
    assert_eq!(avg, -60);
}
