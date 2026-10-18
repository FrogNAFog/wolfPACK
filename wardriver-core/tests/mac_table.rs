//! Host tests for WiGLE throttle and geo-slot semantics in `wardriver_core::mac_table`.

use wardriver_core::mac_table::{
    consider_wigle_ble, update_running_avg_i8, LaneFlags, MacTable, WIGLE_MAX_ROWS_PER_MAC,
};

const LAT: f64 = 37.7749;
const LON: f64 = -122.4194;
/// ~80 m north of [`LAT`] (beyond 50 m revisit).
const LAT_FAR: f64 = 37.7756;
const REVISIT_M: f64 = 50.0;

fn mac(byte: u8) -> [u8; 6] {
    [byte, 0, 0, 0, 0, 0]
}

#[test]
fn update_running_avg_i8_two_samples() {
    assert_eq!(update_running_avg_i8(-90, 1, -84), -87);
}

#[test]
fn geo_revisit_after_slot_release_denies_same_gps_without_move() {
    let mut table = MacTable::new(8, 1);
    let a = mac(1);
    let b = mac(2);

    let first = table.allow(&a, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-90));
    assert!(first.decision.allow_row);

    let second = table.allow(&b, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-90));
    assert!(second.decision.allow_row);
    assert!(!table.get(&a).unwrap().geo_active);

    let revisit = table.allow(&a, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-84));
    assert!(!revisit.decision.allow_row);
}

#[test]
fn first_sighting_sets_avg_and_count() {
    let mut table = MacTable::new(8, 8);
    let m = mac(1);
    assert!(
        table
            .allow(&m, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-90))
            .decision
            .allow_row
    );
    let entry = table.get(&m).unwrap();
    assert_eq!(entry.wigle_wifi_avg_rssi, Some(-90));
    assert_eq!(entry.wigle_wifi_log_count, 1);
}

#[test]
fn wigle_repeat_requires_move_and_weak_must_beat_avg() {
    let mut table = MacTable::new(8, 8);
    let m = mac(3);

    assert!(
        table
            .allow(&m, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-90))
            .decision
            .allow_row
    );

    assert!(
        !table
            .allow(&m, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-84))
            .decision
            .allow_row
    );

    assert!(
        !table
            .allow(
                &m,
                LAT_FAR,
                LON,
                LaneFlags::WIGLE_WIFI,
                REVISIT_M,
                Some(-90)
            )
            .decision
            .allow_row
    );

    assert!(
        table
            .allow(
                &m,
                LAT_FAR,
                LON,
                LaneFlags::WIGLE_WIFI,
                REVISIT_M,
                Some(-84)
            )
            .decision
            .allow_row
    );
}

#[test]
fn wigle_strong_signal_repeat_needs_move_only() {
    let mut table = MacTable::new(8, 8);
    let m = mac(4);

    assert!(
        table
            .allow(&m, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-65))
            .decision
            .allow_row
    );

    assert!(
        table
            .allow(
                &m,
                LAT_FAR,
                LON,
                LaneFlags::WIGLE_WIFI,
                REVISIT_M,
                Some(-70)
            )
            .decision
            .allow_row
    );
}

#[test]
fn wigle_weak_repeat_denied_when_not_above_avg() {
    let mut table = MacTable::new(8, 8);
    let m = mac(7);

    assert!(
        table
            .allow(&m, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-80))
            .decision
            .allow_row
    );

    assert!(
        !table
            .allow(
                &m,
                LAT_FAR,
                LON,
                LaneFlags::WIGLE_WIFI,
                REVISIT_M,
                Some(-88)
            )
            .decision
            .allow_row
    );
}

#[test]
fn wifi_then_ble_increments_both_uniques() {
    let mut table = MacTable::new(8, 8);
    let m = mac(9);

    let wifi = table.allow(&m, LAT, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-50));
    assert!(wifi.decision.new_unique);
    assert!(wifi.decision.allow_row);
    let entry = table.get(&m).unwrap();
    assert!(entry.lanes.has(LaneFlags::WIGLE_WIFI));

    let ble_decision = consider_wigle_ble(Some(entry), LAT, LON, REVISIT_M, -50);
    assert!(ble_decision.new_unique);
    assert!(ble_decision.allow_row, "first BLE sighting at same GPS");

    let ble = table.allow(&m, LAT_FAR, LON, LaneFlags::WIGLE_BLE, REVISIT_M, Some(-50));
    assert!(ble.decision.new_unique);
    assert!(ble.decision.allow_row);

    let lat_farther = LAT_FAR + 0.001;
    let wifi_again = table.allow(
        &m,
        lat_farther,
        LON,
        LaneFlags::WIGLE_WIFI,
        REVISIT_M,
        Some(-50),
    );
    assert!(!wifi_again.decision.new_unique);
    assert!(wifi_again.decision.allow_row, "moved + strong signal");
}

#[test]
fn wigle_cap_shared_across_lanes() {
    let mut table = MacTable::new(16, 16);
    let m = mac(5);
    let mut lat = LAT;
    for _i in 0..WIGLE_MAX_ROWS_PER_MAC {
        assert!(
            table
                .allow(&m, lat, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-50))
                .decision
                .allow_row
        );
        lat += 0.001;
    }
    assert!(
        !table
            .allow(
                &m,
                lat + 0.001,
                LON,
                LaneFlags::WIGLE_BLE,
                REVISIT_M,
                Some(-50)
            )
            .decision
            .allow_row
    );
}

#[test]
fn detection_unaffected_by_wigle_cap() {
    let mut table = MacTable::new(16, 16);
    let m = mac(6);
    let mut lat = LAT;
    for _i in 0..WIGLE_MAX_ROWS_PER_MAC {
        assert!(
            table
                .allow(&m, lat, LON, LaneFlags::WIGLE_WIFI, REVISIT_M, Some(-50))
                .decision
                .allow_row
        );
        lat += 0.001;
    }
    assert!(
        table
            .allow(
                &m,
                lat + 0.001,
                LON,
                LaneFlags::DETECTION_WIFI,
                REVISIT_M,
                None
            )
            .decision
            .allow_row
    );
}
